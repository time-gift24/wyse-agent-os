//! Filesystem-backed agent checkpoint storage.

use std::{
    collections::BTreeSet,
    ops::Bound::{Excluded, Unbounded},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::Value;
use wyse_core::{
    AgentEvent, AgentId, ChatRole, EventSource, HistoryPage, HistoryQuery, RunId, RuntimeEvent,
    StreamEnvelope, TokenUsage, TurnId,
};
use wyse_filesystem::{
    CasExpectation, CasUpdateError, Entry, FILESYSTEM_CAS_RETRIES, FileType, Filesystem,
    FilesystemError, VirtualPath, cas_update,
};

use crate::{
    AGENT_STATE_VERSION, AgentCheckpoint, AgentState, AgentStatus, CheckpointError,
    MAX_HISTORY_PAGE_SIZE,
};

/// Filesystem-backed checkpoint for one agent root.
#[derive(Clone)]
pub struct FilesystemAgentCheckpoint {
    filesystem: Arc<dyn Filesystem>,
    root: VirtualPath,
}

enum CommitSequenceOutcome {
    Committed(AgentState),
    Advanced,
}

impl FilesystemAgentCheckpoint {
    /// Creates a checkpoint rooted at an agent-visible virtual path.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>, root: VirtualPath) -> Self {
        Self { filesystem, root }
    }

    /// Creates the initial agent state and message directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the paths are invalid, the checkpoint already exists,
    /// or the filesystem cannot create it.
    pub async fn initialize(
        &self,
        agent_id: AgentId,
        name: String,
    ) -> Result<AgentState, CheckpointError> {
        let state = AgentState::new(agent_id, name);
        self.filesystem.create_dir(&self.messages_path()?).await?;
        self.filesystem
            .put(
                &self.agent_path()?,
                encode_agent_state(&state)?,
                CasExpectation::Absent,
            )
            .await?;
        Ok(state)
    }

    fn agent_path(&self) -> Result<VirtualPath, CheckpointError> {
        self.child_path("agent.json")
    }

    fn messages_path(&self) -> Result<VirtualPath, CheckpointError> {
        self.child_path("messages")
    }

    fn message_path(&self, seq: u64) -> Result<VirtualPath, CheckpointError> {
        self.child_path(&format!("messages/{seq}.json"))
    }

    fn child_path(&self, suffix: &str) -> Result<VirtualPath, CheckpointError> {
        let path = if self.root.as_str() == "/" {
            format!("/{suffix}")
        } else {
            format!("{}/{suffix}", self.root.as_str())
        };
        VirtualPath::try_from(path.as_str()).map_err(|source| {
            CheckpointError::Filesystem(FilesystemError::InvalidVirtualPath { path, source })
        })
    }

    async fn read_state(&self) -> Result<AgentState, CheckpointError> {
        let path = self.agent_path()?;
        let Some(record) = self.filesystem.get(&path).await? else {
            return Err(CheckpointError::AgentMissing);
        };
        let state = decode_agent_state(&record.entry)?;
        validate_state(&state)?;
        Ok(state)
    }

    async fn read_message(
        &self,
        state: &AgentState,
        seq: u64,
    ) -> Result<Option<StreamEnvelope>, CheckpointError> {
        let Some(record) = self.filesystem.get(&self.message_path(seq)?).await? else {
            return Ok(None);
        };
        let envelope = decode_message(&record.entry).inspect_err(|_| {
            trace_checkpoint_corruption(state, seq);
        })?;
        validate_message(&envelope, state.agent_id, seq, None, None).inspect_err(|_| {
            trace_checkpoint_corruption(state, seq);
        })?;
        Ok(Some(envelope))
    }

    async fn commit_sequence(
        &self,
        expected_previous: u64,
        seq: u64,
    ) -> Result<CommitSequenceOutcome, CheckpointError> {
        let updated_at = Utc::now();
        let attempts = AtomicUsize::new(0);
        let result = cas_update(
            self.filesystem.as_ref(),
            &self.agent_path()?,
            decode_agent_state,
            encode_agent_state,
            |current| {
                attempts.fetch_add(1, Ordering::Relaxed);
                validate_state(current)?;
                if current.last_seq != expected_previous && current.last_seq != seq {
                    return Err(CheckpointError::MessageBeyondFrontier {
                        seq,
                        frontier: current.last_seq,
                    });
                }
                let mut next = current.clone();
                next.last_seq = seq;
                next.updated_at = updated_at;
                Ok(next)
            },
        )
        .await;
        trace_cas_outcome(&result, attempts.load(Ordering::Relaxed), Some(seq));
        match result {
            Ok(state) => Ok(CommitSequenceOutcome::Committed(state)),
            Err(CasUpdateError::Apply(CheckpointError::MessageBeyondFrontier {
                seq: attempted,
                frontier,
            })) if attempted == seq && frontier > seq => Ok(CommitSequenceOutcome::Advanced),
            Err(error) => Err(CheckpointError::from(error)),
        }
    }

    async fn reconcile_frontier(
        &self,
        state: AgentState,
        message_sequences: &BTreeSet<u64>,
    ) -> Result<Option<AgentState>, CheckpointError> {
        let frontier = self
            .validate_integrity_snapshot(&state, message_sequences)
            .await?;
        if !message_sequences.contains(&frontier) {
            return Ok(Some(state));
        }
        let frontier_message = self
            .read_message(&state, frontier)
            .await?
            .ok_or(CheckpointError::MissingCommittedMessage { seq: frontier })?;
        validate_message(
            &frontier_message,
            state.agent_id,
            frontier,
            state.run_id,
            state.turn_id,
        )
        .inspect_err(|_| trace_checkpoint_corruption(&state, frontier))?;

        tracing::info!(
            agent_id = %state.agent_id,
            run_id = ?state.run_id,
            turn_id = ?state.turn_id,
            seq = frontier,
            reconciliation_count = 1_u64,
            "checkpoint frontier reconciliation"
        );
        match self.commit_sequence(state.last_seq, frontier).await? {
            CommitSequenceOutcome::Committed(state) => Ok(Some(state)),
            CommitSequenceOutcome::Advanced => Ok(None),
        }
    }

    async fn list_message_sequences(&self) -> Result<BTreeSet<u64>, CheckpointError> {
        let entries = self.filesystem.list_dir(&self.messages_path()?).await?;
        let mut sequences = BTreeSet::new();
        for entry in entries {
            let seq = parse_message_filename(&entry.file_name)?;
            if entry.file_type != FileType::File || entry.path != self.message_path(seq)? {
                return Err(CheckpointError::InvalidMessageFilename {
                    file_name: entry.file_name,
                });
            }
            sequences.insert(seq);
        }
        Ok(sequences)
    }

    async fn read_integrity_snapshot(
        &self,
    ) -> Result<(AgentState, BTreeSet<u64>), CheckpointError> {
        loop {
            let state = self.read_state().await?;
            let message_sequences = self.list_message_sequences().await?;
            let refreshed = self.read_state().await?;
            if refreshed.last_seq == state.last_seq {
                return Ok((refreshed, message_sequences));
            }
        }
    }

    async fn validate_integrity_snapshot(
        &self,
        state: &AgentState,
        message_sequences: &BTreeSet<u64>,
    ) -> Result<u64, CheckpointError> {
        let frontier = state
            .last_seq
            .checked_add(1)
            .ok_or(CheckpointError::SequenceOverflow)?;
        if let Some(seq) = message_sequences
            .range((Excluded(frontier), Unbounded))
            .next()
        {
            trace_checkpoint_corruption(state, *seq);
            return Err(CheckpointError::MessageBeyondFrontier {
                seq: *seq,
                frontier,
            });
        }
        self.validate_committed_messages(state).await?;
        Ok(frontier)
    }

    async fn validate_committed_messages(&self, state: &AgentState) -> Result<(), CheckpointError> {
        for seq in 1..=state.last_seq {
            if self.read_message(state, seq).await?.is_none() {
                trace_checkpoint_corruption(state, seq);
                return Err(CheckpointError::MissingCommittedMessage { seq });
            }
        }
        Ok(())
    }
}

#[async_trait]
impl AgentCheckpoint for FilesystemAgentCheckpoint {
    async fn load_agent(&self) -> Result<AgentState, CheckpointError> {
        loop {
            let (state, message_sequences) = self.read_integrity_snapshot().await?;
            if let Some(state) = self.reconcile_frontier(state, &message_sequences).await? {
                return Ok(state);
            }
        }
    }

    async fn update_state(
        &self,
        status: AgentStatus,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        usage: TokenUsage,
    ) -> Result<AgentState, CheckpointError> {
        self.load_agent().await?;
        let updated_at = Utc::now();
        let attempts = AtomicUsize::new(0);
        let result = cas_update(
            self.filesystem.as_ref(),
            &self.agent_path()?,
            decode_agent_state,
            encode_agent_state,
            |current| {
                attempts.fetch_add(1, Ordering::Relaxed);
                validate_state(current)?;
                let mut next = current.clone();
                next.status = status;
                next.run_id = run_id;
                next.turn_id = turn_id;
                next.usage = usage;
                next.updated_at = updated_at;
                Ok(next)
            },
        )
        .await;
        trace_cas_outcome(&result, attempts.load(Ordering::Relaxed), None);
        result.map_err(CheckpointError::from)
    }

    async fn append_message(
        &self,
        envelope: StreamEnvelope,
    ) -> Result<StreamEnvelope, CheckpointError> {
        if envelope.business_seq.is_some() {
            return Err(CheckpointError::MessageAlreadySequenced);
        }
        let RuntimeEvent::Agent {
            agent_id,
            event: AgentEvent::Message { turn_id, message },
        } = &envelope.event
        else {
            return Err(CheckpointError::UnexpectedMessageEvent);
        };
        validate_message_role(message.role)?;
        let input_agent_id = *agent_id;
        let run_id = envelope.run_id;
        let turn_id = *turn_id;
        for append_attempt in 1..=FILESYSTEM_CAS_RETRIES {
            let state = self.read_state().await?;
            if input_agent_id != state.agent_id {
                return Err(CheckpointError::AgentMismatch {
                    expected: state.agent_id,
                    actual: input_agent_id,
                });
            }
            let seq = state
                .last_seq
                .checked_add(1)
                .ok_or(CheckpointError::SequenceOverflow)?;
            let beyond = seq
                .checked_add(1)
                .ok_or(CheckpointError::SequenceOverflow)?;
            let mut committed = envelope.clone();
            committed.business_seq = Some(seq);
            validate_message(&committed, state.agent_id, seq, Some(run_id), Some(turn_id))?;

            let existing = self.read_message(&state, seq).await?;
            if self.read_message(&state, beyond).await?.is_some() {
                let refreshed = self.read_state().await?;
                if refreshed.last_seq != state.last_seq {
                    continue;
                }
                trace_checkpoint_corruption(&state, beyond);
                return Err(CheckpointError::MessageBeyondFrontier {
                    seq: beyond,
                    frontier: seq,
                });
            }
            if let Some(existing) = existing {
                validate_message(&existing, state.agent_id, seq, Some(run_id), Some(turn_id))
                    .inspect_err(|_| trace_checkpoint_corruption(&state, seq))?;
                self.commit_sequence(state.last_seq, seq).await?;
                if existing == committed {
                    return Ok(existing);
                }
                tracing::info!(
                    agent_id = %state.agent_id,
                    run_id = %run_id,
                    turn_id = %turn_id,
                    seq,
                    retry_count = append_attempt,
                    "checkpoint append retry"
                );
                continue;
            }

            match self
                .filesystem
                .put(
                    &self.message_path(seq)?,
                    encode_message(&committed)?,
                    CasExpectation::Absent,
                )
                .await
            {
                Ok(_) => {
                    self.commit_sequence(state.last_seq, seq).await?;
                    return Ok(committed);
                }
                Err(FilesystemError::VersionMismatch { .. }) => {
                    continue;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Err(CheckpointError::CasRetriesExhausted)
    }

    async fn history_page(&self, query: HistoryQuery) -> Result<HistoryPage, CheckpointError> {
        let started = std::time::Instant::now();
        if query.limit == 0 || query.limit > MAX_HISTORY_PAGE_SIZE {
            return Err(CheckpointError::InvalidHistoryLimit {
                actual: query.limit,
                maximum: MAX_HISTORY_PAGE_SIZE,
            });
        }
        let state = self.read_state().await?;
        let through_seq = query.through_seq.unwrap_or(state.last_seq);
        if through_seq > state.last_seq {
            return Err(CheckpointError::HistoryBarrierBeyondLast {
                through_seq,
                last_seq: state.last_seq,
            });
        }
        if query.after_seq > through_seq {
            return Err(CheckpointError::InvalidHistoryRange {
                after_seq: query.after_seq,
                through_seq,
            });
        }

        let available = through_seq - query.after_seq;
        let count = available.min(u64::try_from(query.limit).expect("history limit fits u64"));
        let mut events = Vec::with_capacity(usize::try_from(count).expect("page size fits usize"));
        for offset in 1..=count {
            let seq = query
                .after_seq
                .checked_add(offset)
                .ok_or(CheckpointError::SequenceOverflow)?;
            let event = self
                .read_message(&state, seq)
                .await?
                .ok_or(CheckpointError::MissingCommittedMessage { seq })?;
            events.push(event);
        }
        let next_front_seq = events
            .last()
            .and_then(StreamEnvelope::business_seq)
            .unwrap_or(query.after_seq);
        let page = HistoryPage {
            through_seq,
            events,
            next_front_seq,
            has_more: next_front_seq < through_seq,
        };
        tracing::info!(
            agent_id = %state.agent_id,
            run_id = ?state.run_id,
            turn_id = ?state.turn_id,
            seq = page.next_front_seq,
            event_count = page.events.len(),
            latency_micros = started.elapsed().as_micros(),
            "checkpoint history page"
        );
        Ok(page)
    }
}

fn decode_agent_state(entry: &Entry) -> Result<AgentState, CheckpointError> {
    serde_json::from_slice(entry.contents()).map_err(CheckpointError::DecodeState)
}

fn encode_agent_state(state: &AgentState) -> Result<Entry, CheckpointError> {
    serde_json::to_vec(state)
        .map(Entry::new)
        .map_err(CheckpointError::Encode)
}

fn decode_message(entry: &Entry) -> Result<StreamEnvelope, CheckpointError> {
    let value: Value =
        serde_json::from_slice(entry.contents()).map_err(CheckpointError::DecodeMessage)?;
    validate_strict_message_json(&value).map_err(CheckpointError::DecodeMessage)?;
    serde_json::from_value(value).map_err(CheckpointError::DecodeMessage)
}

fn encode_message(envelope: &StreamEnvelope) -> Result<Entry, CheckpointError> {
    serde_json::to_vec(envelope)
        .map(Entry::new)
        .map_err(CheckpointError::Encode)
}

fn validate_state(state: &AgentState) -> Result<(), CheckpointError> {
    if state.state_version != AGENT_STATE_VERSION {
        return Err(CheckpointError::UnsupportedStateVersion {
            version: state.state_version,
        });
    }
    Ok(())
}

fn parse_message_filename(file_name: &str) -> Result<u64, CheckpointError> {
    let Some(number) = file_name.strip_suffix(".json") else {
        return Err(CheckpointError::InvalidMessageFilename {
            file_name: file_name.to_owned(),
        });
    };
    let seq = number
        .parse::<u64>()
        .ok()
        .filter(|seq| *seq != 0 && number == seq.to_string())
        .ok_or_else(|| CheckpointError::InvalidMessageFilename {
            file_name: file_name.to_owned(),
        })?;
    Ok(seq)
}

fn validate_message(
    envelope: &StreamEnvelope,
    expected_agent_id: AgentId,
    path_seq: u64,
    expected_run_id: Option<RunId>,
    expected_turn_id: Option<TurnId>,
) -> Result<(), CheckpointError> {
    if let Some(expected) = expected_run_id
        && envelope.run_id != expected
    {
        return Err(CheckpointError::RunMismatch {
            expected,
            actual: envelope.run_id,
        });
    }
    if let EventSource::Agent { agent_id, .. } = &envelope.source
        && *agent_id != expected_agent_id
    {
        return Err(CheckpointError::AgentMismatch {
            expected: expected_agent_id,
            actual: *agent_id,
        });
    }
    let RuntimeEvent::Agent { agent_id, event } = &envelope.event else {
        return Err(CheckpointError::UnexpectedMessageEvent);
    };
    if *agent_id != expected_agent_id {
        return Err(CheckpointError::AgentMismatch {
            expected: expected_agent_id,
            actual: *agent_id,
        });
    }
    let AgentEvent::Message { turn_id, message } = event else {
        return Err(CheckpointError::UnexpectedMessageEvent);
    };
    validate_message_role(message.role)?;
    if envelope.business_seq != Some(path_seq) {
        return Err(CheckpointError::MessageSequenceMismatch {
            path_seq,
            event_seq: envelope.business_seq.unwrap_or_default(),
        });
    }
    if let Some(expected) = expected_turn_id
        && *turn_id != expected
    {
        return Err(CheckpointError::TurnMismatch {
            expected,
            actual: *turn_id,
        });
    }
    Ok(())
}

fn validate_message_role(role: ChatRole) -> Result<(), CheckpointError> {
    match role {
        ChatRole::User | ChatRole::Assistant | ChatRole::Tool => Ok(()),
        role => Err(CheckpointError::InvalidMessageRole { role }),
    }
}

fn validate_strict_message_json(value: &Value) -> Result<(), serde_json::Error> {
    let envelope = strict_object(value)?;
    strict_keys(
        envelope,
        &["business_seq", "run_id", "timestamp", "source", "event"],
        &[
            "business_seq",
            "run_id",
            "timestamp",
            "source",
            "event",
            "metadata",
        ],
    )?;
    validate_strict_source(&envelope["source"])?;

    let runtime_event = strict_object(&envelope["event"])?;
    strict_keys(runtime_event, &["type", "data"], &["type", "data"])?;
    if runtime_event["type"] != "agent" {
        return Err(strict_json_error());
    }
    let runtime_data = strict_object(&runtime_event["data"])?;
    strict_keys(runtime_data, &["agent_id", "event"], &["agent_id", "event"])?;

    let agent_event = strict_object(&runtime_data["event"])?;
    strict_keys(agent_event, &["type", "data"], &["type", "data"])?;
    if agent_event["type"] != "message" {
        return Err(strict_json_error());
    }
    let message_data = strict_object(&agent_event["data"])?;
    strict_keys(
        message_data,
        &["turn_id", "message"],
        &["turn_id", "message"],
    )?;
    validate_strict_chat_message(&message_data["message"])
}

fn validate_strict_source(value: &Value) -> Result<(), serde_json::Error> {
    let source = strict_object(value)?;
    let Some(source_type) = source.get("type").and_then(Value::as_str) else {
        return Err(strict_json_error());
    };
    match source_type {
        "run" => strict_keys(source, &["type"], &["type"]),
        "node" => strict_keys(source, &["type", "node_id"], &["type", "node_id"]),
        "agent" => strict_keys(
            source,
            &["type", "node_id", "agent_id"],
            &["type", "node_id", "agent_id"],
        ),
        _ => Err(strict_json_error()),
    }
}

fn validate_strict_chat_message(value: &Value) -> Result<(), serde_json::Error> {
    let message = strict_object(value)?;
    strict_keys(
        message,
        &["role", "content"],
        &[
            "role",
            "content",
            "tool_calls",
            "reasoning_content",
            "tool_call_id",
        ],
    )?;
    let content = strict_object(&message["content"])?;
    strict_keys(content, &["type", "data"], &["type", "data"])?;
    if !matches!(content["type"].as_str(), Some("text" | "json")) {
        return Err(strict_json_error());
    }
    if let Some(tool_calls) = message.get("tool_calls") {
        let Some(tool_calls) = tool_calls.as_array() else {
            return Err(strict_json_error());
        };
        for tool_call in tool_calls {
            strict_keys(
                strict_object(tool_call)?,
                &["call_id", "name", "arguments"],
                &["call_id", "name", "arguments"],
            )?;
        }
    }
    Ok(())
}

fn strict_object(value: &Value) -> Result<&serde_json::Map<String, Value>, serde_json::Error> {
    value.as_object().ok_or_else(strict_json_error)
}

fn strict_keys(
    object: &serde_json::Map<String, Value>,
    required: &[&str],
    allowed: &[&str],
) -> Result<(), serde_json::Error> {
    if required.iter().any(|key| !object.contains_key(*key))
        || object.keys().any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(strict_json_error());
    }
    Ok(())
}

fn strict_json_error() -> serde_json::Error {
    serde_json::Error::io(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "invalid strict checkpoint message shape",
    ))
}

fn trace_cas_outcome(
    result: &Result<AgentState, CasUpdateError<CheckpointError>>,
    attempt_count: usize,
    seq: Option<u64>,
) {
    let state = result.as_ref().ok();
    let agent_id = state.map(|state| state.agent_id);
    let run_id = state.and_then(|state| state.run_id);
    let turn_id = state.and_then(|state| state.turn_id);
    if attempt_count > 1 {
        tracing::warn!(
            agent_id = ?agent_id,
            run_id = ?run_id,
            turn_id = ?turn_id,
            seq = ?seq,
            retry_count = attempt_count - 1,
            "checkpoint cas retry"
        );
    }
    if matches!(result, Err(CasUpdateError::RetriesExhausted)) {
        tracing::error!(
            agent_id = ?agent_id,
            run_id = ?run_id,
            turn_id = ?turn_id,
            seq = ?seq,
            attempt_count,
            exhaustion_count = 1_u64,
            "checkpoint cas retries exhausted"
        );
    }
}

fn trace_checkpoint_corruption(state: &AgentState, seq: u64) {
    tracing::error!(
        agent_id = %state.agent_id,
        run_id = ?state.run_id,
        turn_id = ?state.turn_id,
        seq,
        corruption_count = 1_u64,
        "checkpoint corruption"
    );
}
