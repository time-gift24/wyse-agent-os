mod support;

use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use support::MemoryCasFilesystem;
use wyse_core::{
    AgentEvent, AgentId, ChatMessage, ChatRole, EventSource, HistoryQuery, RunId, RuntimeEvent,
    StreamEnvelope, TokenUsage, TurnId,
};
use wyse_filesystem::{Entry, FILESYSTEM_CAS_RETRIES, VirtualPath};
use wyse_store::{AgentState, AgentStatus, AgentStore, FilesystemAgentStore, StoreError};

fn message_envelope(agent_id: AgentId, run_id: RunId, turn_id: TurnId) -> StreamEnvelope {
    StreamEnvelope {
        business_seq: None,
        run_id,
        timestamp: Utc::now(),
        source: EventSource::Run,
        event: RuntimeEvent::Agent {
            agent_id,
            event: AgentEvent::Message {
                turn_id,
                message: ChatMessage::user("message"),
            },
        },
        metadata: BTreeMap::new(),
    }
}

fn sequenced_message_envelope(
    agent_id: AgentId,
    run_id: RunId,
    turn_id: TurnId,
    seq: u64,
) -> StreamEnvelope {
    let mut envelope = message_envelope(agent_id, run_id, turn_id);
    envelope.business_seq = Some(seq);
    envelope
}

fn json_entry<T: serde::Serialize>(value: &T) -> Entry {
    Entry::new(serde_json::to_vec(value).expect("serialize fixture entry"))
}

fn event_sequences(events: &[StreamEnvelope]) -> Vec<u64> {
    events
        .iter()
        .map(|event| event.business_seq().expect("message sequence"))
        .collect()
}

async fn append_messages(store: &FilesystemAgentStore, count: usize) {
    let state = store.load_agent().await.expect("load agent");
    let agent_id = state.agent_id;
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    for index in 0..count {
        let appended = store
            .append_message(message_envelope(agent_id, run_id, turn_id))
            .await
            .expect("append message");
        let expected_seq =
            state.last_seq + u64::try_from(index).expect("message index fits u64") + 1;
        assert_eq!(appended.business_seq(), Some(expected_seq));
    }
}

#[tokio::test]
async fn initialize_and_append_create_exact_files_and_advance_last_seq() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();

    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let first = store
        .append_message(message_envelope(agent_id, RunId::new(), TurnId::new()))
        .await
        .expect("append");

    assert_eq!(first.business_seq(), Some(1));
    assert!(filesystem.exists("/agents/a/agent.json"));
    assert!(filesystem.exists("/agents/a/messages/1.json"));
    let stored: StreamEnvelope = serde_json::from_slice(
        filesystem
            .entry("/agents/a/messages/1.json")
            .expect("stored message")
            .contents(),
    )
    .expect("decode stored message");
    assert_eq!(stored.business_seq(), Some(1));
    assert_eq!(stored, first);
    assert_eq!(store.load_agent().await.expect("state").last_seq, 1);
}

#[tokio::test]
async fn complete_iteration_advances_and_persists_usage() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem, root);
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let usage = TokenUsage {
        input_tokens: 2,
        output_tokens: 3,
        total_tokens: 5,
    };
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    store
        .update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("start run");

    let completed = store
        .complete_iteration(run_id, turn_id, 0, usage)
        .await
        .expect("complete iteration");

    assert_eq!(completed.next_iteration, 1);
    assert_eq!(completed.usage, usage);
    let persisted = store.load_agent().await.expect("load persisted state");
    assert_eq!(persisted.next_iteration, 1);
    assert_eq!(persisted.usage, usage);
}

#[tokio::test]
async fn complete_iteration_rejects_non_running_state_without_writing() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let before = filesystem
        .entry("/agents/a/agent.json")
        .expect("agent state")
        .contents()
        .to_vec();

    let error = store
        .complete_iteration(RunId::new(), TurnId::new(), 0, TokenUsage::default())
        .await
        .expect_err("idle state is not running");

    assert!(matches!(
        error,
        StoreError::AgentNotRunning {
            actual: AgentStatus::Idle
        }
    ));
    assert_eq!(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent state")
            .contents(),
        before
    );
}

#[tokio::test]
async fn complete_iteration_rejects_run_mismatch_without_writing() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let other_run_id = RunId::new();
    let turn_id = TurnId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    store
        .update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("start run");
    let before = filesystem
        .entry("/agents/a/agent.json")
        .expect("agent state")
        .contents()
        .to_vec();

    let error = store
        .complete_iteration(other_run_id, turn_id, 0, TokenUsage::default())
        .await
        .expect_err("run mismatch");

    assert!(matches!(
        error,
        StoreError::RunMismatch { expected, actual }
            if expected == run_id && actual == other_run_id
    ));
    assert_eq!(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent state")
            .contents(),
        before
    );
}

#[tokio::test]
async fn complete_iteration_rejects_turn_mismatch_without_writing() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let other_turn_id = TurnId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    store
        .update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("start run");
    let before = filesystem
        .entry("/agents/a/agent.json")
        .expect("agent state")
        .contents()
        .to_vec();

    let error = store
        .complete_iteration(run_id, other_turn_id, 0, TokenUsage::default())
        .await
        .expect_err("turn mismatch");

    assert!(matches!(
        error,
        StoreError::TurnMismatch { expected, actual }
            if expected == turn_id && actual == other_turn_id
    ));
    assert_eq!(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent state")
            .contents(),
        before
    );
}

#[tokio::test]
async fn complete_iteration_rejects_iteration_mismatch_without_writing() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    store
        .update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("start run");
    let before = filesystem
        .entry("/agents/a/agent.json")
        .expect("agent state")
        .contents()
        .to_vec();

    let error = store
        .complete_iteration(run_id, turn_id, 1, TokenUsage::default())
        .await
        .expect_err("iteration mismatch");

    assert!(matches!(
        error,
        StoreError::IterationMismatch {
            expected: 0,
            actual: 1
        }
    ));
    assert_eq!(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent state")
            .contents(),
        before
    );
}

#[tokio::test]
async fn complete_iteration_rejects_iteration_overflow_without_writing() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.next_iteration = u64::MAX;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&state));
    let before = filesystem
        .entry("/agents/a/agent.json")
        .expect("agent state")
        .contents()
        .to_vec();

    let error = store
        .complete_iteration(run_id, turn_id, u64::MAX, TokenUsage::default())
        .await
        .expect_err("iteration overflow");

    assert!(matches!(error, StoreError::IterationOverflow));
    assert_eq!(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent state")
            .contents(),
        before
    );
}

#[tokio::test]
async fn state_update_resets_new_run_iteration_and_preserves_terminal_iteration() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem, root);
    let agent_id = AgentId::new();
    let old_run_id = RunId::new();
    let old_turn_id = TurnId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    store
        .update_state(
            AgentStatus::Running,
            Some(old_run_id),
            Some(old_turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("start old run");
    store
        .complete_iteration(old_run_id, old_turn_id, 0, TokenUsage::default())
        .await
        .expect("complete old iteration");
    let run_id = RunId::new();
    let turn_id = TurnId::new();

    let started = store
        .update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("start new run");
    assert_eq!(started.next_iteration, 0);
    store
        .complete_iteration(run_id, turn_id, 0, TokenUsage::default())
        .await
        .expect("complete new iteration");

    for status in [
        AgentStatus::Finished,
        AgentStatus::Failed,
        AgentStatus::Cancelled,
    ] {
        let terminal = store
            .update_state(status, Some(run_id), Some(turn_id), TokenUsage::default())
            .await
            .expect("store terminal state");
        assert_eq!(terminal.next_iteration, 1);
    }
}

#[tokio::test]
async fn append_rejects_an_already_sequenced_message() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let mut envelope = message_envelope(agent_id, RunId::new(), TurnId::new());
    envelope.business_seq = Some(7);

    let error = store
        .append_message(envelope)
        .await
        .expect_err("sequenced input");

    assert!(matches!(error, StoreError::MessageAlreadySequenced));
    assert_eq!(store.load_agent().await.expect("state").last_seq, 0);
    assert!(!filesystem.exists("/agents/a/messages/1.json"));
}

#[tokio::test]
async fn append_rejects_a_system_message_before_writing() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let mut envelope = message_envelope(agent_id, RunId::new(), TurnId::new());
    let RuntimeEvent::Agent {
        event: AgentEvent::Message { message, .. },
        ..
    } = &mut envelope.event
    else {
        panic!("message fixture");
    };
    *message = ChatMessage::system("system prompt");

    let error = store
        .append_message(envelope)
        .await
        .expect_err("system message role");

    assert!(matches!(
        error,
        StoreError::InvalidMessageRole {
            role: ChatRole::System
        }
    ));
    assert_eq!(store.load_agent().await.expect("state").last_seq, 0);
    assert!(!filesystem.exists("/agents/a/messages/1.json"));
}

#[tokio::test]
async fn load_rejects_a_committed_system_message() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    state.last_seq = 1;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&state));
    let mut envelope = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let RuntimeEvent::Agent {
        event: AgentEvent::Message { message, .. },
        ..
    } = &mut envelope.event
    else {
        panic!("message fixture");
    };
    *message = ChatMessage::system("system prompt");
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&envelope));

    let error = store
        .load_agent()
        .await
        .expect_err("committed system message role");

    assert!(matches!(
        error,
        StoreError::InvalidMessageRole {
            role: ChatRole::System
        }
    ));
}

#[tokio::test]
async fn load_rejects_an_uncommitted_system_frontier_without_advancing_state() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let mut envelope = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let RuntimeEvent::Agent {
        event: AgentEvent::Message { message, .. },
        ..
    } = &mut envelope.event
    else {
        panic!("message fixture");
    };
    *message = ChatMessage::system("system prompt");
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&envelope));

    let error = store
        .load_agent()
        .await
        .expect_err("uncommitted system frontier role");

    assert!(matches!(
        error,
        StoreError::InvalidMessageRole {
            role: ChatRole::System
        }
    ));
    let persisted: AgentState = serde_json::from_slice(
        filesystem
            .entry("/agents/a/agent.json")
            .expect("agent entry")
            .contents(),
    )
    .expect("agent state");
    assert_eq!(persisted.last_seq, 0);
}

#[tokio::test]
async fn append_rejects_a_message_for_a_different_agent() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let other_agent_id = AgentId::new();

    let error = store
        .append_message(message_envelope(
            other_agent_id,
            RunId::new(),
            TurnId::new(),
        ))
        .await
        .expect_err("agent mismatch");

    assert!(matches!(
        error,
        StoreError::AgentMismatch { expected, actual }
            if expected == agent_id && actual == other_agent_id
    ));
    assert_eq!(store.load_agent().await.expect("state").last_seq, 0);
    assert!(!filesystem.exists("/agents/a/messages/1.json"));
}

#[tokio::test]
async fn append_rejects_a_non_message_agent_event() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let turn_id = TurnId::new();
    let mut envelope = message_envelope(agent_id, RunId::new(), turn_id);
    envelope.event = RuntimeEvent::Agent {
        agent_id,
        event: AgentEvent::Started { turn_id },
    };

    let error = store
        .append_message(envelope)
        .await
        .expect_err("non-message event");

    assert!(matches!(error, StoreError::UnexpectedMessageEvent));
    assert_eq!(store.load_agent().await.expect("state").last_seq, 0);
    assert!(!filesystem.exists("/agents/a/messages/1.json"));
}

#[tokio::test]
async fn load_reconciles_one_valid_frontier_without_rewriting_it() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    assert_eq!(state.last_seq, 0);
    let envelope = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&envelope));
    let message_version = filesystem
        .entry_version("/agents/a/messages/1.json")
        .expect("message version");

    let reconciled = store.load_agent().await.expect("reconcile frontier");

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
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let second = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 2);
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&second));

    let error = store
        .load_agent()
        .await
        .expect_err("discontiguous extra message");

    assert!(matches!(
        error,
        StoreError::MessageBeyondFrontier {
            seq: 2,
            frontier: 1
        }
    ));
}

#[tokio::test]
async fn load_rejects_a_third_message_beyond_the_single_frontier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let first = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let third = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 3);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.insert_entry("/agents/a/messages/3.json", json_entry(&third));

    let error = store
        .load_agent()
        .await
        .expect_err("message beyond single frontier");

    assert!(matches!(
        error,
        StoreError::MessageBeyondFrontier {
            seq: 3,
            frontier: 1
        }
    ));
}

#[tokio::test]
async fn load_rejects_noncanonical_message_filenames() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let first = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/01.json", json_entry(&first));

    let error = store.load_agent().await.expect_err("noncanonical filename");

    assert!(matches!(
        error,
        StoreError::InvalidMessageFilename { file_name } if file_name == "01.json"
    ));
}

#[tokio::test]
async fn append_retry_returns_an_identical_uncommitted_frontier_without_duplication() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let requested = message_envelope(agent_id, run_id, turn_id);
    let mut envelope = requested.clone();
    envelope.business_seq = Some(1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&envelope));
    let message_version = filesystem
        .entry_version("/agents/a/messages/1.json")
        .expect("message version");

    let appended = store.append_message(requested).await.expect("retry append");

    assert_eq!(appended, envelope);
    assert_eq!(store.load_agent().await.expect("state").last_seq, 1);
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
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let frontier = sequenced_message_envelope(agent_id, run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&frontier));
    let frontier_version = filesystem
        .entry_version("/agents/a/messages/1.json")
        .expect("frontier version");
    let mut requested = message_envelope(agent_id, run_id, turn_id);
    requested
        .metadata
        .insert("request".to_owned(), serde_json::json!(true));

    let appended = store
        .append_message(requested)
        .await
        .expect("append after frontier");

    assert_eq!(appended.business_seq(), Some(2));
    assert_eq!(store.load_agent().await.expect("state").last_seq, 2);
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
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let frontier_run_id = RunId::new();
    let turn_id = TurnId::new();
    let frontier = sequenced_message_envelope(agent_id, frontier_run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&frontier));
    let requested_run_id = RunId::new();

    let error = store
        .append_message(message_envelope(agent_id, requested_run_id, turn_id))
        .await
        .expect_err("run mismatch");

    assert!(matches!(
        error,
        StoreError::RunMismatch { expected, actual }
            if expected == requested_run_id && actual == frontier_run_id
    ));
    assert_eq!(store.load_agent().await.expect("reconcile").last_seq, 1);
}

#[tokio::test]
async fn append_rejects_discontiguous_message_before_advancing_frontier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let second = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 2);
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&second));

    let error = store
        .append_message(message_envelope(agent_id, RunId::new(), TurnId::new()))
        .await
        .expect_err("discontiguous message");

    assert!(matches!(
        error,
        StoreError::MessageBeyondFrontier {
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
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    state.last_seq = 2;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&state));
    let first = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.remove_entry("/agents/a/messages/2.json");

    let error = store.load_agent().await.expect_err("missing message");

    assert!(matches!(
        error,
        StoreError::MissingCommittedMessage { seq: 2 }
    ));
}

#[tokio::test]
async fn load_rejects_message_filename_body_sequence_mismatch() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    state.last_seq = 2;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&state));
    let first = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let mismatched = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 3);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&mismatched));

    let error = store.load_agent().await.expect_err("sequence mismatch");

    assert!(matches!(
        error,
        StoreError::MessageSequenceMismatch {
            path_seq: 2,
            event_seq: 3
        }
    ));
}

#[tokio::test]
async fn load_rejects_message_for_a_different_agent() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let other_agent_id = AgentId::new();
    let frontier = sequenced_message_envelope(other_agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&frontier));

    let error = store.load_agent().await.expect_err("agent mismatch");

    assert!(matches!(
        error,
        StoreError::AgentMismatch { expected, actual }
            if expected == agent_id && actual == other_agent_id
    ));
}

#[tokio::test]
async fn load_rejects_unknown_message_json_fields() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let envelope = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    let mut value = serde_json::to_value(envelope).expect("serialize envelope");
    value
        .as_object_mut()
        .expect("envelope object")
        .insert("owner_id".to_owned(), serde_json::json!("unexpected"));
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&value));

    let error = store.load_agent().await.expect_err("unknown field");

    assert!(matches!(error, StoreError::DecodeMessage(_)));
}

#[tokio::test]
async fn state_update_retry_preserves_concurrently_advanced_last_seq() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut advanced_state: AgentState = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    advanced_state.last_seq = 1;
    let first = sequenced_message_envelope(agent_id, RunId::new(), TurnId::new(), 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.fail_next_version_write();

    let update = tokio::spawn({
        let store = store.clone();
        async move {
            store
                .update_state(
                    AgentStatus::Finished,
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

    assert_eq!(updated.status, AgentStatus::Finished);
    assert_eq!(updated.last_seq, 1);
}

#[tokio::test]
async fn state_update_reconciles_the_previous_run_frontier_before_replacing_identity() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let old_run_id = RunId::new();
    let old_turn_id = TurnId::new();
    store
        .update_state(
            AgentStatus::Running,
            Some(old_run_id),
            Some(old_turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("store old identity");
    let old_frontier = sequenced_message_envelope(agent_id, old_run_id, old_turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&old_frontier));
    let new_run_id = RunId::new();
    let new_turn_id = TurnId::new();

    let updated = store
        .update_state(
            AgentStatus::Running,
            Some(new_run_id),
            Some(new_turn_id),
            TokenUsage::default(),
        )
        .await
        .expect("store new identity after reconciliation");

    assert_eq!(updated.last_seq, 1);
    assert_eq!(updated.run_id, Some(new_run_id));
    assert_eq!(updated.turn_id, Some(new_turn_id));
    let page = store
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: Some(1),
            limit: 1,
        })
        .await
        .expect("old frontier is committed and loadable");
    assert_eq!(page.events, [old_frontier]);
    let appended = store
        .append_message(message_envelope(agent_id, new_run_id, new_turn_id))
        .await
        .expect("append new-run message");
    assert_eq!(appended.business_seq(), Some(2));
}

#[tokio::test]
async fn load_retries_when_frontier_cas_observes_a_later_valid_state() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut latest_state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let first = sequenced_message_envelope(agent_id, run_id, turn_id, 1);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&first));
    filesystem.pause_next_version_write();

    let load = tokio::spawn({
        let store = store.clone();
        async move { store.load_agent().await }
    });
    filesystem.wait_for_version_write_pause().await;
    let second = sequenced_message_envelope(agent_id, run_id, turn_id, 2);
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&second));
    latest_state.last_seq = 2;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&latest_state));
    filesystem.resume_version_write();

    let loaded = load
        .await
        .expect("load task")
        .expect("load retries after state advance");
    let page = store
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
    let store = FilesystemAgentStore::new(filesystem, root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 3).await;

    let first = store
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: None,
            limit: 2,
        })
        .await
        .expect("first page");
    append_messages(&store, 1).await;
    let second = store
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
    let store = FilesystemAgentStore::new(filesystem, root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");

    for limit in [0, 257] {
        let error = store
            .history_page(HistoryQuery {
                after_seq: 0,
                through_seq: None,
                limit,
            })
            .await
            .expect_err("invalid limit");
        assert!(matches!(
            error,
            StoreError::InvalidHistoryLimit { actual, maximum: 256 }
                if actual == limit
        ));
    }
}

#[tokio::test]
async fn pagination_rejects_front_beyond_barrier() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem, root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 2).await;

    let error = store
        .history_page(HistoryQuery {
            after_seq: 2,
            through_seq: Some(1),
            limit: 1,
        })
        .await
        .expect_err("front beyond barrier");

    assert!(matches!(
        error,
        StoreError::InvalidHistoryRange {
            after_seq: 2,
            through_seq: 1
        }
    ));
}

#[tokio::test]
async fn pagination_rejects_barrier_beyond_last_committed_sequence() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem, root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 2).await;

    let error = store
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: Some(3),
            limit: 1,
        })
        .await
        .expect_err("barrier beyond last");

    assert!(matches!(
        error,
        StoreError::HistoryBarrierBeyondLast {
            through_seq: 3,
            last_seq: 2
        }
    ));
}

#[tokio::test]
async fn pagination_rejects_missing_path_inside_committed_range() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 2).await;
    filesystem.remove_entry("/agents/a/messages/2.json");

    let error = store
        .history_page(HistoryQuery {
            after_seq: 0,
            through_seq: None,
            limit: 2,
        })
        .await
        .expect_err("missing committed path");

    assert!(matches!(
        error,
        StoreError::MissingCommittedMessage { seq: 2 }
    ));
}

#[tokio::test]
async fn pagination_reads_nine_and_ten_in_numeric_order() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem, root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 10).await;

    let page = store
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
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    store
        .initialize(AgentId::new(), "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 5).await;
    filesystem.reset_read_counts();

    let page = store
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
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    append_messages(&store, 10).await;
    filesystem.reset_read_counts();

    store
        .append_message(message_envelope(agent_id, RunId::new(), TurnId::new()))
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
async fn append_stops_after_the_filesystem_cas_retry_limit() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    filesystem.fail_absent_writes();

    let error = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        store.append_message(message_envelope(agent_id, RunId::new(), TurnId::new())),
    )
    .await
    .expect("append terminates")
    .expect_err("append retry exhaustion");

    assert!(matches!(error, StoreError::CasRetriesExhausted));
    assert_eq!(filesystem.absent_write_attempts(), FILESYSTEM_CAS_RETRIES);
}

#[tokio::test]
async fn append_retries_when_one_beyond_belongs_to_an_advanced_state() {
    let filesystem = Arc::new(MemoryCasFilesystem::default());
    let root = VirtualPath::try_from("/agents/a").expect("valid root");
    let store = FilesystemAgentStore::new(filesystem.clone(), root);
    let agent_id = AgentId::new();
    let mut advanced_state = store
        .initialize(agent_id, "a".to_owned())
        .await
        .expect("initialize");
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let requested = message_envelope(agent_id, run_id, turn_id);
    filesystem.pause_next_read("/agents/a/messages/2.json");

    let append = tokio::spawn({
        let store = store.clone();
        async move { store.append_message(requested).await }
    });
    filesystem.wait_for_read_pause().await;
    let committed = sequenced_message_envelope(agent_id, run_id, turn_id, 1);
    let frontier = sequenced_message_envelope(agent_id, run_id, turn_id, 2);
    filesystem.insert_entry("/agents/a/messages/1.json", json_entry(&committed));
    filesystem.insert_entry("/agents/a/messages/2.json", json_entry(&frontier));
    advanced_state.last_seq = 1;
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&advanced_state));
    filesystem.resume_read();

    let appended = append
        .await
        .expect("append task")
        .expect("stale append retries");

    assert_eq!(appended.business_seq(), Some(3));
    assert_eq!(store.load_agent().await.expect("state").last_seq, 3);
    assert!(filesystem.exists("/agents/a/messages/3.json"));
}
