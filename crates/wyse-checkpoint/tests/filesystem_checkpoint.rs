mod support;

use std::{collections::BTreeMap, sync::Arc};

use chrono::{DateTime, Utc};
use support::MemoryCasFilesystem;
use wyse_checkpoint::{
    AgentCheckpoint, AgentState, AgentStatus, CheckpointError, FilesystemAgentCheckpoint,
};
use wyse_core::{
    AgentEvent, AgentId, ChatMessage, EventSource, HistoryQuery, RunId, RuntimeEvent,
    StreamEnvelope, TokenUsage, TurnId,
};
use wyse_filesystem::{Entry, VirtualPath};

fn message_envelope(agent_id: AgentId, run_id: RunId, turn_id: TurnId, seq: u64) -> StreamEnvelope {
    StreamEnvelope {
        run_id,
        timestamp: DateTime::<Utc>::UNIX_EPOCH,
        source: EventSource::Run,
        event: RuntimeEvent::Agent {
            agent_id,
            event: AgentEvent::Message {
                seq,
                turn_id,
                message: ChatMessage::user(format!("message {seq}")),
            },
        },
        metadata: BTreeMap::new(),
    }
}

fn json_entry<T: serde::Serialize>(value: &T) -> Entry {
    Entry::new(serde_json::to_vec(value).expect("serialize fixture entry"))
}

fn event_sequences(events: &[StreamEnvelope]) -> Vec<u64> {
    events
        .iter()
        .map(|event| event.event.business_seq().expect("message sequence"))
        .collect()
}

async fn append_messages(checkpoint: &FilesystemAgentCheckpoint, count: usize) {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    for index in 0..count {
        checkpoint
            .append_message(
                run_id,
                turn_id,
                DateTime::<Utc>::UNIX_EPOCH,
                EventSource::Run,
                ChatMessage::user(format!("message {index}")),
                BTreeMap::new(),
            )
            .await
            .expect("append message");
    }
}

#[tokio::test]
async fn initialize_and_append_create_exact_files_and_advance_last_seq() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();

    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let first = checkpoint
        .append_message(
            RunId::new(),
            TurnId::new(),
            Utc::now(),
            EventSource::Run,
            ChatMessage::user("hello"),
            BTreeMap::new(),
        )
        .await
        .expect("append");

    assert_eq!(first.event.business_seq(), Some(1));
    assert!(filesystem.exists("/agents/a/agent.json"));
    assert!(filesystem.exists("/agents/a/messages/1.json"));
    assert_eq!(checkpoint.load_agent().await.expect("state").last_seq, 1);
}

#[tokio::test]
async fn load_reconciles_one_valid_frontier_without_rewriting_it() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let state = checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    assert_eq!(state.last_seq, 0);
    let envelope = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&envelope));
    let message_version = filesystem
        .entry_version("/agents/a/messages/1.json")
        .expect("message version");

    let reconciled = checkpoint.load_agent().await.expect("reconcile frontier");

    assert_eq!(reconciled.last_seq, 1);
    assert_eq!(
        filesystem.entry_version("/agents/a/messages/1.json"),
        Some(message_version)
    );
}

#[tokio::test]
async fn load_rejects_a_discontiguous_second_message_without_a_frontier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let second = message_envelope(agent_id, RunId::new(), TurnId::new(), 2);
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&second));

    let error = checkpoint
        .load_agent()
        .await
        .expect_err("discontiguous extra message");

    assert!(matches!(
        error,
        CheckpointError::MessageBeyondFrontier {
            seq: 2,
            frontier: 1
        }
    ));
}

#[tokio::test]
async fn load_rejects_a_third_message_beyond_the_single_frontier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let first = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let third = message_envelope(agent_id, RunId::new(), TurnId::new(), 3);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.insert_entry("/agents/a/messages/3.json", json_entry(&third));

    let error = checkpoint
        .load_agent()
        .await
        .expect_err("message beyond single frontier");

    assert!(matches!(
        error,
        CheckpointError::MessageBeyondFrontier {
            seq: 3,
            frontier: 1
        }
    ));
}

#[tokio::test]
async fn load_rejects_noncanonical_message_filenames() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let first = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/01.json", json_entry(&first));

    let error = checkpoint
        .load_agent()
        .await
        .expect_err("noncanonical filename");

    assert!(matches!(
        error,
        CheckpointError::InvalidMessageFilename { file_name } if file_name == "01.json"
    ));
}

#[tokio::test]
async fn append_retry_returns_an_identical_uncommitted_frontier_without_duplication() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let envelope = message_envelope(agent_id, run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&envelope));
    let message_version = filesystem
        .entry_version("/agents/a/messages/1.json")
        .expect("message version");

    let appended = checkpoint
        .append_message(
            run_id,
            turn_id,
            DateTime::<Utc>::UNIX_EPOCH,
            EventSource::Run,
            ChatMessage::user("message 1"),
            BTreeMap::new(),
        )
        .await
        .expect("retry append");

    assert_eq!(appended, envelope);
    assert_eq!(checkpoint.load_agent().await.expect("state").last_seq, 1);
    assert!(!filesystem.exists("/agents/a/messages/2.json"));
    assert_eq!(
        filesystem.entry_version("/agents/a/messages/1.json"),
        Some(message_version)
    );
}

#[tokio::test]
async fn append_reconciles_a_different_frontier_then_retries_at_the_next_sequence() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let frontier = message_envelope(agent_id, run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&frontier));
    let frontier_version = filesystem
        .entry_version("/agents/a/messages/1.json")
        .expect("frontier version");

    let appended = checkpoint
        .append_message(
            run_id,
            turn_id,
            DateTime::<Utc>::UNIX_EPOCH,
            EventSource::Run,
            ChatMessage::user("different"),
            BTreeMap::new(),
        )
        .await
        .expect("append after frontier");

    assert_eq!(appended.event.business_seq(), Some(2));
    assert_eq!(checkpoint.load_agent().await.expect("state").last_seq, 2);
    assert!(filesystem.exists("/agents/a/messages/2.json"));
    assert_eq!(
        filesystem.entry_version("/agents/a/messages/1.json"),
        Some(frontier_version)
    );
}

#[tokio::test]
async fn append_rejects_an_uncommitted_frontier_from_a_different_run() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let frontier_run_id = RunId::new();
    let turn_id = TurnId::new();
    let frontier = message_envelope(agent_id, frontier_run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&frontier));
    let requested_run_id = RunId::new();

    let error = checkpoint
        .append_message(
            requested_run_id,
            turn_id,
            DateTime::<Utc>::UNIX_EPOCH,
            EventSource::Run,
            ChatMessage::user("different"),
            BTreeMap::new(),
        )
        .await
        .expect_err("run mismatch");

    assert!(matches!(
        error,
        CheckpointError::RunMismatch { expected, actual }
            if expected == requested_run_id && actual == frontier_run_id
    ));
    assert_eq!(
        checkpoint.load_agent().await.expect("reconcile").last_seq,
        1
    );
}

#[tokio::test]
async fn append_rejects_discontiguous_message_before_advancing_frontier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let second = message_envelope(agent_id, RunId::new(), TurnId::new(), 2);
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&second));

    let error = checkpoint
        .append_message(
            RunId::new(),
            TurnId::new(),
            DateTime::<Utc>::UNIX_EPOCH,
            EventSource::Run,
            ChatMessage::user("must not persist"),
            BTreeMap::new(),
        )
        .await
        .expect_err("discontiguous message");

    assert!(matches!(
        error,
        CheckpointError::MessageBeyondFrontier {
            seq: 2,
            frontier: 1
        }
    ));
    let state: AgentState = serde_json::from_slice(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent entry")
            .contents(),
    )
    .expect("agent state");
    assert_eq!(state.last_seq, 0);
    assert!(!filesystem.exists("/agents/a/messages/1.json"));
}

#[tokio::test]
async fn load_rejects_missing_committed_message() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut state = checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    state.last_seq = 2;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&state));
    let first = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.remove_entry("/agents/a/messages/2.json");

    let error = checkpoint.load_agent().await.expect_err("missing message");

    assert!(matches!(
        error,
        CheckpointError::MissingCommittedMessage { seq: 2 }
    ));
}

#[tokio::test]
async fn load_rejects_message_filename_body_sequence_mismatch() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut state = checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    state.last_seq = 2;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&state));
    let first = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let mismatched = message_envelope(agent_id, RunId::new(), TurnId::new(), 3);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&mismatched));

    let error = checkpoint
        .load_agent()
        .await
        .expect_err("sequence mismatch");

    assert!(matches!(
        error,
        CheckpointError::MessageSequenceMismatch {
            path_seq: 2,
            event_seq: 3
        }
    ));
}

#[tokio::test]
async fn load_rejects_message_for_a_different_agent() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let other_agent_id = AgentId::new();
    let frontier = message_envelope(other_agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&frontier));

    let error = checkpoint.load_agent().await.expect_err("agent mismatch");

    assert!(matches!(
        error,
        CheckpointError::AgentMismatch { expected, actual }
            if expected == agent_id && actual == other_agent_id
    ));
}

#[tokio::test]
async fn load_rejects_unknown_message_json_fields() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let envelope = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let mut value = serde_json::to_value(envelope).expect("serialize envelope");
    value
        .as_object_mut()
        .expect("envelope object")
        .insert("owner_id".to_owned(), serde_json::json!("unexpected"));
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&value));

    let error = checkpoint.load_agent().await.expect_err("unknown field");

    assert!(matches!(error, CheckpointError::DecodeMessage(_)));
}

#[tokio::test]
async fn state_update_retry_preserves_concurrently_advanced_last_seq() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut advanced_state: AgentState = checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    advanced_state.last_seq = 1;
    let first = message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.fail_next_version_write();

    let update = tokio::spawn({
        let checkpoint = checkpoint.clone();
        async move {
            checkpoint
                .update_state(
                    AgentStatus::Running,
                    Some(RunId::new()),
                    Some(TurnId::new()),
                    TokenUsage::default(),
                )
                .await
        }
    });
    while filesystem.version_write_failure_pending() {
        tokio::task::yield_now().await;
    }
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&advanced_state));
    let updated = update
        .await
        .expect("state update task")
        .expect("state update retries");

    assert_eq!(updated.status, AgentStatus::Running);
    assert_eq!(updated.last_seq, 1);
}

#[tokio::test]
async fn load_retries_when_frontier_cas_observes_a_later_valid_state() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut latest_state = checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let first = message_envelope(agent_id, run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.pause_next_version_write();

    let load = tokio::spawn({
        let checkpoint = checkpoint.clone();
        async move { checkpoint.load_agent().await }
    });
    filesystem.wait_for_version_write_pause().await;
    let second = message_envelope(agent_id, run_id, turn_id, 2);
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&second));
    latest_state.last_seq = 2;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&latest_state));
    filesystem.resume_version_write();

    let loaded = load
        .await
        .expect("load task")
        .expect("load retries after state advance");
    let page = checkpoint
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: Some(2),
            limit: 2,
        })
        .await
        .expect("latest valid history");

    assert_eq!(loaded.last_seq, 2);
    assert_eq!(event_sequences(&page.events), [1, 2]);
}

#[tokio::test]
async fn pagination_keeps_the_first_page_barrier_after_a_later_append() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem, root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 3).await;

    let first = checkpoint
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: None,
            limit: 2,
        })
        .await
        .expect("first page");
    append_messages(&checkpoint, 1).await;
    let second = checkpoint
        .history_page(HistoryQuery {
            after_seq: first.next_front_seq,
            through_seq: Some(first.through_seq),
            limit: 2,
        })
        .await
        .expect("second page");

    assert_eq!(first.through_seq, 3);
    assert_eq!(event_sequences(&first.events), [1, 2]);
    assert!(first.has_more);
    assert_eq!(second.through_seq, 3);
    assert_eq!(event_sequences(&second.events), [3]);
    assert!(!second.has_more);
}

#[tokio::test]
async fn pagination_rejects_zero_and_oversized_limits() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem, root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");

    for limit in [0, 257] {
        let error = checkpoint
            .history_page(HistoryQuery {
                after_seq: 0,
                through_seq: None,
                limit,
            })
            .await
            .expect_err("invalid limit");
        assert!(matches!(
            error,
            CheckpointError::InvalidHistoryLimit { actual, maximum: 256 }
                if actual == limit
        ));
    }
}

#[tokio::test]
async fn pagination_rejects_front_beyond_barrier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem, root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 2).await;

    let error = checkpoint
        .history_page(HistoryQuery {
            after_seq: 2,
            through_seq: Some(1),
            limit: 1,
        })
        .await
        .expect_err("front beyond barrier");

    assert!(matches!(
        error,
        CheckpointError::InvalidHistoryRange {
            after_seq: 2,
            through_seq: 1
        }
    ));
}

#[tokio::test]
async fn pagination_rejects_barrier_beyond_last_committed_sequence() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem, root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 2).await;

    let error = checkpoint
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: Some(3),
            limit: 1,
        })
        .await
        .expect_err("barrier beyond last");

    assert!(matches!(
        error,
        CheckpointError::HistoryBarrierBeyondLast {
            through_seq: 3,
            last_seq: 2
        }
    ));
}

#[tokio::test]
async fn pagination_rejects_missing_path_inside_committed_range() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 2).await;
    filesystem.remove_entry("/agents/a/messages/2.json");

    let error = checkpoint
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: None,
            limit: 2,
        })
        .await
        .expect_err("missing committed path");

    assert!(matches!(
        error,
        CheckpointError::MissingCommittedMessage { seq: 2 }
    ));
}

#[tokio::test]
async fn pagination_reads_nine_and_ten_in_numeric_order() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem, root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 10).await;

    let page = checkpoint
        .history_page(HistoryQuery {
            after_seq: 8,
            through_seq: Some(10),
            limit: 2,
        })
        .await
        .expect("numeric page");

    assert_eq!(event_sequences(&page.events), [9, 10]);
}

#[tokio::test]
async fn pagination_reads_only_the_requested_message_paths() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 5).await;
    filesystem.reset_read_counts();

    let page = checkpoint
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: Some(5),
            limit: 1,
        })
        .await
        .expect("single-message page");

    assert_eq!(event_sequences(&page.events), [1]);
    assert_eq!(filesystem.read_count("/agents/a/agent.json"), 1);
    assert_eq!(filesystem.read_count("/agents/a/messages/1.json"), 1);
    for seq in 2..=5 {
        assert_eq!(
            filesystem.read_count(&format!("/agents/a/messages/{seq}.json")),
            0
        );
    }
    assert_eq!(filesystem.list_count(), 0);
}

#[tokio::test]
async fn append_reads_only_state_and_the_constant_size_frontier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    checkpoint
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&checkpoint, 10).await;
    filesystem.reset_read_counts();

    checkpoint
        .append_message(
            RunId::new(),
            TurnId::new(),
            DateTime::<Utc>::UNIX_EPOCH,
            EventSource::Run,
            ChatMessage::user("message 11"),
            BTreeMap::new(),
        )
        .await
        .expect("append after long history");

    assert_eq!(filesystem.list_count(), 0);
    for seq in 1..=10 {
        assert_eq!(
            filesystem.read_count(&format!("/agents/a/messages/{seq}.json")),
            0
        );
    }
    assert_eq!(filesystem.read_count("/agents/a/messages/11.json"), 1);
    assert_eq!(filesystem.read_count("/agents/a/messages/12.json"), 1);
}

#[tokio::test]
async fn append_retries_when_one_beyond_belongs_to_an_advanced_state() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let checkpoint = FilesystemAgentCheckpoint::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut advanced_state = checkpoint
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    filesystem.pause_next_read("/agents/a/messages/2.json");

    let append = tokio::spawn({
        let checkpoint = checkpoint.clone();
        async move {
            checkpoint
                .append_message(
                    run_id,
                    turn_id,
                    DateTime::<Utc>::UNIX_EPOCH,
                    EventSource::Run,
                    ChatMessage::user("requested"),
                    BTreeMap::new(),
                )
                .await
        }
    });
    filesystem.wait_for_read_pause().await;
    let committed = message_envelope(agent_id, run_id, turn_id, 1);
    let frontier = message_envelope(agent_id, run_id, turn_id, 2);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&committed));
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&frontier));
    advanced_state.last_seq = 1;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&advanced_state));
    filesystem.resume_read();

    let appended = append
        .await
        .expect("append task")
        .expect("stale append retries");

    assert_eq!(appended.event.business_seq(), Some(3));
    assert_eq!(checkpoint.load_agent().await.expect("state").last_seq, 3);
    assert!(filesystem.exists("/agents/a/messages/3.json"));
}
