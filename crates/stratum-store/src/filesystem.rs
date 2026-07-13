//! Filesystem-backed agent store storage.

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
use stratum_core::{
    AgentEvent, AgentId, ChatRole, EventSource, HistoryPage, HistoryQuery, ModelConfig, RunId,
    RuntimeEvent, StreamEnvelope, TokenUsage, TurnId,
};
use stratum_filesystem::{
    CasExpectation, CasUpdateError, Entry, FILESYSTEM_CAS_RETRIES, FileType, Filesystem,
    FilesystemError, VirtualPath, cas_update,
};

use crate::state::LEGACY_AGENT_STATE_VERSION;
use crate::{
    AGENT_STATE_VERSION, AgentState, AgentStatus, AgentStore, MAX_HISTORY_PAGE_SIZE, StoreError,
};

/// Filesystem-backed store for one agent root.
#[derive(Clone)]
pub struct FilesystemAgentStore {
    filesystem: Arc<dyn Filesystem>,
    root: VirtualPath,
}

enum CommitSequenceOutcome {
    Committed(AgentState),
    Advanced,
}

#[derive(Debug, thiserror::Error)]
enum ModelConfigMigrationError {
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("model configuration is already persisted")]
    AlreadyPersisted,
}

impl FilesystemAgentStore {
    /// Creates a store rooted at an agent-visible virtual path.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>, root: VirtualPath) -> Self {
        Self { filesystem, root }
    }

    /// Creates the initial agent state and message directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the paths are invalid, the store already exists,
    /// or the filesystem cannot create it.
    pub async fn initialize(
        &self,
        agent_id: AgentId,
        name: String,
    ) -> Result<AgentState, StoreError> {
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

    /// Creates the initial host-configured agent state and message directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the paths are invalid, the store already exists,
    /// or the filesystem cannot create it.
    pub async fn initialize_with_model_config(
        &self,
        agent_id: AgentId,
        name: String,
        model_config: ModelConfig,
    ) -> Result<AgentState, StoreError> {
        let state = AgentState::new_configured(agent_id, name, model_config);
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

    /// Persists a model configuration for a legacy state that lacks one.
    ///
    /// # Errors
    ///
    /// Returns an error when the state is invalid or the compare-and-swap update cannot be
    /// committed.
    pub async fn write_model_config_if_missing(
        &self,
        model_config: ModelConfig,
    ) -> Result<AgentState, StoreError> {
        let current = self.read_state_for_model_config().await?;
        if current.model_config.is_some() {
            return Ok(current);
        }

        let attempts = AtomicUsize::new(0);
        let result = cas_update(
            self.filesystem.as_ref(),
            &self.agent_path()?,
            decode_agent_state_for_model_config_migration,
            encode_agent_state_for_model_config_migration,
            |current| {
                attempts.fetch_add(1, Ordering::Relaxed);
                validate_persisted_state(current)?;
                if current.model_config.is_some() {
                    return Err(ModelConfigMigrationError::AlreadyPersisted);
                }
                let mut next = current.clone();
                next.state_version = AGENT_STATE_VERSION;
                next.model_config = Some(model_config.clone());
                Ok(next)
            },
        )
        .await;
        trace_cas_outcome(&result, attempts.load(Ordering::Relaxed), None);
        match result {
            Ok(state) => Ok(state),
            Err(CasUpdateError::Apply(ModelConfigMigrationError::AlreadyPersisted)) => {
                self.read_state_for_model_config().await
            }
            Err(CasUpdateError::Apply(ModelConfigMigrationError::Store(error))) => Err(error),
            Err(CasUpdateError::CasUnsupported) => Err(StoreError::CasUnsupported),
            Err(CasUpdateError::Timeout) => Err(StoreError::CasTimeout),
            Err(CasUpdateError::RetriesExhausted) => Err(StoreError::CasRetriesExhausted),
            Err(CasUpdateError::Filesystem(error)) => Err(StoreError::Filesystem(error)),
        }
    }

    fn agent_path(&self) -> Result<VirtualPath, StoreError> {
        self.child_path("agent.json")
    }

    fn messages_path(&self) -> Result<VirtualPath, StoreError> {
        self.child_path("messages")
    }

    fn message_path(&self, seq: u64) -> Result<VirtualPath, StoreError> {
        self.child_path(&format!("messages/{seq}.json"))
    }

    fn child_path(&self, suffix: &str) -> Result<VirtualPath, StoreError> {
        let path = if self.root.as_str() == "/" {
            format!("/{suffix}")
        } else {
            format!("{}/{suffix}", self.root.as_str())
        };
        VirtualPath::try_from(path.as_str()).map_err(|source| {
            StoreError::Filesystem(FilesystemError::InvalidVirtualPath { path, source })
        })
    }

    async fn read_state(&self) -> Result<AgentState, StoreError> {
        let state = self.read_state_for_model_config().await?;
        validate_runtime_state(&state)?;
        Ok(state)
    }

    async fn read_state_for_model_config(&self) -> Result<AgentState, StoreError> {
        let path = self.agent_path()?;
        let Some(record) = self.filesystem.get(&path).await? else {
            return Err(StoreError::AgentMissing);
        };
        let state = decode_agent_state(&record.entry)?;
        validate_persisted_state(&state)?;
        Ok(state)
    }

    async fn read_message(
        &self,
        state: &AgentState,
        seq: u64,
    ) -> Result<Option<StreamEnvelope>, StoreError> {
        let Some(record) = self.filesystem.get(&self.message_path(seq)?).await? else {
            return Ok(None);
        };
        let envelope = decode_message(&record.entry).inspect_err(|_| {
            trace_store_corruption(state, seq);
        })?;
        validate_message(&envelope, state.agent_id, seq, None, None).inspect_err(|_| {
            trace_store_corruption(state, seq);
        })?;
        Ok(Some(envelope))
    }

    async fn commit_sequence(
        &self,
        expected_previous: u64,
        seq: u64,
    ) -> Result<CommitSequenceOutcome, StoreError> {
        let updated_at = Utc::now();
        let attempts = AtomicUsize::new(0);
        let result = cas_update(
            self.filesystem.as_ref(),
            &self.agent_path()?,
            decode_agent_state,
            encode_agent_state,
            |current| {
                attempts.fetch_add(1, Ordering::Relaxed);
                validate_persisted_state(current)?;
                if current.last_seq != expected_previous && current.last_seq != seq {
                    return Err(StoreError::MessageBeyondFrontier {
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
            Err(CasUpdateError::Apply(StoreError::MessageBeyondFrontier {
                seq: attempted,
                frontier,
            })) if attempted == seq && frontier > seq => Ok(CommitSequenceOutcome::Advanced),
            Err(error) => Err(StoreError::from(error)),
        }
    }

    async fn reconcile_frontier(
        &self,
        state: AgentState,
        message_sequences: &BTreeSet<u64>,
    ) -> Result<Option<AgentState>, StoreError> {
        let frontier = self
            .validate_integrity_snapshot(&state, message_sequences)
            .await?;
        if !message_sequences.contains(&frontier) {
            return Ok(Some(state));
        }
        let frontier_message = self
            .read_message(&state, frontier)
            .await?
            .ok_or(StoreError::MissingCommittedMessage { seq: frontier })?;
        validate_message(
            &frontier_message,
            state.agent_id,
            frontier,
            state.run_id,
            state.turn_id,
        )
        .inspect_err(|_| trace_store_corruption(&state, frontier))?;

        tracing::info!(
            agent_id = %state.agent_id,
            run_id = ?state.run_id,
            turn_id = ?state.turn_id,
            seq = frontier,
            reconciliation_count = 1_u64,
            "store frontier reconciliation"
        );
        match self.commit_sequence(state.last_seq, frontier).await? {
            CommitSequenceOutcome::Committed(state) => Ok(Some(state)),
            CommitSequenceOutcome::Advanced => Ok(None),
        }
    }

    async fn list_message_sequences(&self) -> Result<BTreeSet<u64>, StoreError> {
        let entries = self.filesystem.list_dir(&self.messages_path()?).await?;
        let mut sequences = BTreeSet::new();
        for entry in entries {
            let seq = parse_message_filename(&entry.file_name)?;
            if entry.file_type != FileType::File || entry.path != self.message_path(seq)? {
                return Err(StoreError::InvalidMessageFilename {
                    file_name: entry.file_name,
                });
            }
            sequences.insert(seq);
        }
        Ok(sequences)
    }

    async fn read_integrity_snapshot(
        &self,
        allow_legacy: bool,
    ) -> Result<(AgentState, BTreeSet<u64>), StoreError> {
        loop {
            let state = if allow_legacy {
                self.read_state_for_model_config().await?
            } else {
                self.read_state().await?
            };
            let message_sequences = self.list_message_sequences().await?;
            let refreshed = if allow_legacy {
                self.read_state_for_model_config().await?
            } else {
                self.read_state().await?
            };
            if refreshed.last_seq == state.last_seq {
                return Ok((refreshed, message_sequences));
            }
        }
    }

    async fn validate_integrity_snapshot(
        &self,
        state: &AgentState,
        message_sequences: &BTreeSet<u64>,
    ) -> Result<u64, StoreError> {
        let frontier = state
            .last_seq
            .checked_add(1)
            .ok_or(StoreError::SequenceOverflow)?;
        if let Some(seq) = message_sequences
            .range((Excluded(frontier), Unbounded))
            .next()
        {
            trace_store_corruption(state, *seq);
            return Err(StoreError::MessageBeyondFrontier {
                seq: *seq,
                frontier,
            });
        }
        self.validate_committed_messages(state).await?;
        Ok(frontier)
    }

    async fn validate_committed_messages(&self, state: &AgentState) -> Result<(), StoreError> {
        for seq in 1..=state.last_seq {
            if self.read_message(state, seq).await?.is_none() {
                trace_store_corruption(state, seq);
                return Err(StoreError::MissingCommittedMessage { seq });
            }
        }
        Ok(())
    }

    async fn load_agent_for_model_config(&self) -> Result<AgentState, StoreError> {
        loop {
            let (state, message_sequences) = self.read_integrity_snapshot(true).await?;
            if let Some(state) = self.reconcile_frontier(state, &message_sequences).await? {
                return Ok(state);
            }
        }
    }
}

#[async_trait]
impl AgentStore for FilesystemAgentStore {
    async fn load_agent(&self) -> Result<AgentState, StoreError> {
        loop {
            let (state, message_sequences) = self.read_integrity_snapshot(false).await?;
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
    ) -> Result<AgentState, StoreError> {
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
                validate_runtime_state(current)?;
                if status == AgentStatus::Running
                    && current.status == AgentStatus::Running
                    && current.run_id != run_id
                {
                    return Err(StoreError::RunningRunConflict {
                        current: current.run_id,
                        attempted: run_id,
                    });
                }
                let mut next = current.clone();
                if status == AgentStatus::Running && current.run_id != run_id {
                    next.next_iteration = 0;
                }
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
        result.map_err(StoreError::from)
    }

    async fn start_turn(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        model_config: ModelConfig,
    ) -> Result<AgentState, StoreError> {
        self.load_agent_for_model_config().await?;
        let updated_at = Utc::now();
        let attempts = AtomicUsize::new(0);
        let result = cas_update(
            self.filesystem.as_ref(),
            &self.agent_path()?,
            decode_agent_state,
            encode_agent_state,
            |current| {
                attempts.fetch_add(1, Ordering::Relaxed);
                validate_persisted_state(current)?;
                if current.status == AgentStatus::Running && current.run_id != Some(run_id) {
                    return Err(StoreError::RunningRunConflict {
                        current: current.run_id,
                        attempted: Some(run_id),
                    });
                }
                let mut next = current.clone();
                next.state_version = AGENT_STATE_VERSION;
                next.status = AgentStatus::Running;
                next.run_id = Some(run_id);
                next.turn_id = Some(turn_id);
                next.next_iteration = 0;
                next.usage = TokenUsage::default();
                next.model_config = Some(model_config.clone());
                next.updated_at = updated_at;
                Ok(next)
            },
        )
        .await;
        trace_cas_outcome(&result, attempts.load(Ordering::Relaxed), None);
        result.map_err(StoreError::from)
    }

    async fn complete_iteration(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        iteration: u64,
        usage: TokenUsage,
    ) -> Result<AgentState, StoreError> {
        let updated_at = Utc::now();
        let attempts = AtomicUsize::new(0);
        let result = cas_update(
            self.filesystem.as_ref(),
            &self.agent_path()?,
            decode_agent_state,
            encode_agent_state,
            |current| {
                attempts.fetch_add(1, Ordering::Relaxed);
                validate_runtime_state(current)?;
                if current.status != AgentStatus::Running {
                    return Err(StoreError::AgentNotRunning {
                        actual: current.status,
                    });
                }
                if current.run_id != Some(run_id) {
                    return Err(StoreError::RunMismatch {
                        expected: current.run_id.unwrap_or(run_id),
                        actual: run_id,
                    });
                }
                if current.turn_id != Some(turn_id) {
                    return Err(StoreError::TurnMismatch {
                        expected: current.turn_id.unwrap_or(turn_id),
                        actual: turn_id,
                    });
                }
                if current.next_iteration != iteration {
                    return Err(StoreError::IterationMismatch {
                        expected: current.next_iteration,
                        actual: iteration,
                    });
                }
                let mut next = current.clone();
                next.next_iteration = iteration
                    .checked_add(1)
                    .ok_or(StoreError::IterationOverflow)?;
                next.usage = usage;
                next.updated_at = updated_at;
                Ok(next)
            },
        )
        .await;
        trace_cas_outcome(&result, attempts.load(Ordering::Relaxed), None);
        result.map_err(StoreError::from)
    }

    async fn append_message(&self, envelope: StreamEnvelope) -> Result<StreamEnvelope, StoreError> {
        if envelope.business_seq.is_some() {
            return Err(StoreError::MessageAlreadySequenced);
        }
        let RuntimeEvent::Agent {
            agent_id,
            event: AgentEvent::Message { turn_id, message },
        } = &envelope.event
        else {
            return Err(StoreError::UnexpectedMessageEvent);
        };
        validate_message_role(message.role)?;
        let input_agent_id = *agent_id;
        let run_id = envelope.run_id;
        let turn_id = *turn_id;
        for append_attempt in 1..=FILESYSTEM_CAS_RETRIES {
            let state = self.read_state().await?;
            if input_agent_id != state.agent_id {
                return Err(StoreError::AgentMismatch {
                    expected: state.agent_id,
                    actual: input_agent_id,
                });
            }
            let seq = state
                .last_seq
                .checked_add(1)
                .ok_or(StoreError::SequenceOverflow)?;
            let beyond = seq.checked_add(1).ok_or(StoreError::SequenceOverflow)?;
            let mut committed = envelope.clone();
            committed.business_seq = Some(seq);
            validate_message(&committed, state.agent_id, seq, Some(run_id), Some(turn_id))?;

            let existing = self.read_message(&state, seq).await?;
            if self.read_message(&state, beyond).await?.is_some() {
                let refreshed = self.read_state().await?;
                if refreshed.last_seq != state.last_seq {
                    continue;
                }
                trace_store_corruption(&state, beyond);
                return Err(StoreError::MessageBeyondFrontier {
                    seq: beyond,
                    frontier: seq,
                });
            }
            if let Some(existing) = existing {
                validate_message(&existing, state.agent_id, seq, Some(run_id), Some(turn_id))
                    .inspect_err(|_| trace_store_corruption(&state, seq))?;
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
                    "store append retry"
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
        Err(StoreError::CasRetriesExhausted)
    }

    async fn history_page(&self, query: HistoryQuery) -> Result<HistoryPage, StoreError> {
        let started = std::time::Instant::now();
        if query.limit == 0 || query.limit > MAX_HISTORY_PAGE_SIZE {
            return Err(StoreError::InvalidHistoryLimit {
                actual: query.limit,
                maximum: MAX_HISTORY_PAGE_SIZE,
            });
        }
        let state = self.read_state().await?;
        let through_seq = query.through_seq.unwrap_or(state.last_seq);
        if through_seq > state.last_seq {
            return Err(StoreError::HistoryBarrierBeyondLast {
                through_seq,
                last_seq: state.last_seq,
            });
        }
        if query.after_seq > through_seq {
            return Err(StoreError::InvalidHistoryRange {
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
                .ok_or(StoreError::SequenceOverflow)?;
            let event = self
                .read_message(&state, seq)
                .await?
                .ok_or(StoreError::MissingCommittedMessage { seq })?;
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
            "store history page"
        );
        Ok(page)
    }
}

fn decode_agent_state(entry: &Entry) -> Result<AgentState, StoreError> {
    serde_json::from_slice(entry.contents()).map_err(StoreError::DecodeState)
}

fn decode_agent_state_for_model_config_migration(
    entry: &Entry,
) -> Result<AgentState, ModelConfigMigrationError> {
    decode_agent_state(entry).map_err(ModelConfigMigrationError::from)
}

fn encode_agent_state(state: &AgentState) -> Result<Entry, StoreError> {
    serde_json::to_vec(state)
        .map(Entry::new)
        .map_err(StoreError::Encode)
}

fn encode_agent_state_for_model_config_migration(
    state: &AgentState,
) -> Result<Entry, ModelConfigMigrationError> {
    encode_agent_state(state).map_err(ModelConfigMigrationError::from)
}

fn decode_message(entry: &Entry) -> Result<StreamEnvelope, StoreError> {
    let value: Value =
        serde_json::from_slice(entry.contents()).map_err(StoreError::DecodeMessage)?;
    validate_strict_message_json(&value).map_err(StoreError::DecodeMessage)?;
    serde_json::from_value(value).map_err(StoreError::DecodeMessage)
}

fn encode_message(envelope: &StreamEnvelope) -> Result<Entry, StoreError> {
    serde_json::to_vec(envelope)
        .map(Entry::new)
        .map_err(StoreError::Encode)
}

fn validate_persisted_state(state: &AgentState) -> Result<(), StoreError> {
    match (state.state_version, &state.model_config) {
        (AGENT_STATE_VERSION, Some(_)) | (LEGACY_AGENT_STATE_VERSION, None) => Ok(()),
        (AGENT_STATE_VERSION, None) => Err(StoreError::MissingModelConfig),
        (version, _) => Err(StoreError::UnsupportedStateVersion { version }),
    }
}

fn validate_runtime_state(state: &AgentState) -> Result<(), StoreError> {
    validate_persisted_state(state)?;
    if state.model_config.is_none() {
        return Err(StoreError::MissingModelConfig);
    }
    Ok(())
}

fn parse_message_filename(file_name: &str) -> Result<u64, StoreError> {
    let Some(number) = file_name.strip_suffix(".json") else {
        return Err(StoreError::InvalidMessageFilename {
            file_name: file_name.to_owned(),
        });
    };
    let seq = number
        .parse::<u64>()
        .ok()
        .filter(|seq| *seq != 0 && number == seq.to_string())
        .ok_or_else(|| StoreError::InvalidMessageFilename {
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
) -> Result<(), StoreError> {
    if let Some(expected) = expected_run_id
        && envelope.run_id != expected
    {
        return Err(StoreError::RunMismatch {
            expected,
            actual: envelope.run_id,
        });
    }
    if let EventSource::Agent { agent_id, .. } = &envelope.source
        && *agent_id != expected_agent_id
    {
        return Err(StoreError::AgentMismatch {
            expected: expected_agent_id,
            actual: *agent_id,
        });
    }
    let RuntimeEvent::Agent { agent_id, event } = &envelope.event else {
        return Err(StoreError::UnexpectedMessageEvent);
    };
    if *agent_id != expected_agent_id {
        return Err(StoreError::AgentMismatch {
            expected: expected_agent_id,
            actual: *agent_id,
        });
    }
    let AgentEvent::Message { turn_id, message } = event else {
        return Err(StoreError::UnexpectedMessageEvent);
    };
    validate_message_role(message.role)?;
    if envelope.business_seq != Some(path_seq) {
        return Err(StoreError::MessageSequenceMismatch {
            path_seq,
            event_seq: envelope.business_seq.unwrap_or_default(),
        });
    }
    if let Some(expected) = expected_turn_id
        && *turn_id != expected
    {
        return Err(StoreError::TurnMismatch {
            expected,
            actual: *turn_id,
        });
    }
    Ok(())
}

fn validate_message_role(role: ChatRole) -> Result<(), StoreError> {
    match role {
        ChatRole::User | ChatRole::Assistant | ChatRole::Tool => Ok(()),
        role => Err(StoreError::InvalidMessageRole { role }),
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
        "invalid strict store message shape",
    ))
}

fn trace_cas_outcome<E>(
    result: &Result<AgentState, CasUpdateError<E>>,
    attempt_count: usize,
    seq: Option<u64>,
) where
    E: std::error::Error + 'static,
{
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
            "store cas retry"
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
            "store cas retries exhausted"
        );
    }
}

fn trace_store_corruption(state: &AgentState, seq: u64) {
    tracing::error!(
        agent_id = %state.agent_id,
        run_id = ?state.run_id,
        turn_id = ?state.turn_id,
        seq,
        corruption_count = 1_u64,
        "store corruption"
    );
}
