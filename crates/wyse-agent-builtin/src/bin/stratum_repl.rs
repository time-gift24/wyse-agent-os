use std::{
    io::{BufRead, Write},
    path::PathBuf,
    sync::Arc,
};

use clap::Parser;
use futures_util::StreamExt;
use thiserror::Error;
use wyse_agent::{Agent, AgentError};
use wyse_agent_builtin::build_default_agent;
use wyse_core::{
    AgentEvent, AgentId, ApprovalDecision, ApprovalId, ChatContent, ChatMessage, ChatRole,
    DangerLevel, ModelId, ModelIdParseError, ReplayStart, RuntimeEvent, ToolKind,
};
use wyse_filesystem::{
    Filesystem, FilesystemError, LocalFilesystem, LocalFilesystemConfig, VirtualPath,
    VirtualPathError,
};
use wyse_infra::{
    EventStream, EventStreamBus, EventStreamBusError, event_stream_bus::InMemoryEventStreamBus,
};
use wyse_llm::{
    ApiKey, DeepSeekModel, DeepSeekProvider, DeepSeekThinking, LlmError, LlmProvider,
    OpenAICompatibleProvider,
};
use wyse_store::{AgentStatus, AgentStore, FilesystemAgentStore, StoreError, StoreEventStreamBus};
use wyse_tools::{BuiltinToolRegistry, EchoTool, ToolError, ToolPermissionMode, ToolRegistry};

const CONFIG_PATH: &str = "config.toml";
const DEFAULT_AGENT_NAME: &str = "default-agent";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

#[derive(Parser)]
#[command(name = "stratum-repl")]
struct Args {
    #[arg(long)]
    resume: Option<AgentId>,
    #[arg(long)]
    debug: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    stratum: StratumConfig,
    openai: Option<ProviderConfig>,
    deepseek: Option<ProviderConfig>,
}

impl Config {
    fn read() -> Result<Self, ReplError> {
        let contents = std::fs::read_to_string(CONFIG_PATH)?;
        toml::from_str(&contents).map_err(ReplError::from)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StratumConfig {
    storage_root: PathBuf,
    model: ModelId,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderConfig {
    api_key: String,
}

struct Session {
    agent_id: AgentId,
    agent: Agent,
    store: Arc<dyn AgentStore>,
    bus: Arc<dyn EventStreamBus>,
    storage_root: PathBuf,
}

#[derive(Debug, Error)]
enum ReplError {
    #[error("failed to parse command line arguments")]
    Args(#[from] clap::Error),
    #[error("failed to read configuration")]
    Io(#[from] std::io::Error),
    #[error("failed to parse configuration")]
    Toml(#[from] toml::de::Error),
    #[error("invalid model id")]
    ModelId(#[from] ModelIdParseError),
    #[error("agent operation failed")]
    Agent(#[from] AgentError),
    #[error("agent store operation failed")]
    Store(#[from] StoreError),
    #[error("filesystem operation failed")]
    Filesystem(#[from] FilesystemError),
    #[error("invalid virtual path")]
    VirtualPath(#[from] VirtualPathError),
    #[error("event stream bus operation failed")]
    EventStreamBus(#[from] EventStreamBusError),
    #[error("llm operation failed")]
    Llm(#[from] LlmError),
    #[error("tool operation failed")]
    Tool(#[from] ToolError),
    #[error("json encoding failed")]
    Json(#[from] serde_json::Error),
    #[error("event stream closed before a terminal agent event")]
    EventStreamClosed,
    #[error("unsupported provider: {provider}")]
    UnsupportedProvider { provider: String },
    #[error("unsupported model: {model}")]
    UnsupportedModel { model: ModelId },
    #[error("missing provider configuration: {provider}")]
    MissingProviderConfiguration { provider: &'static str },
}

#[tokio::main]
async fn main() -> Result<(), ReplError> {
    let args = Args::parse();
    let config = Config::read()?;
    let agent_id = args.resume.unwrap_or_else(AgentId::new);
    let session = compose_session(&config, agent_id, args.resume.is_none()).await?;
    let mut output = std::io::stdout();
    writeln!(output, "agent id: {}", session.agent_id)?;
    writeln!(output, "storage root: {}", session.storage_root.display())?;

    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    restore_session(&session, &mut input, args.debug, &mut output).await?;
    let mut line = String::new();
    loop {
        write!(output, "> ")?;
        output.flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break;
        }
        if line.trim().is_empty() {
            continue;
        }
        let turn_input = line.trim_end_matches(['\r', '\n']);
        if turn_input == "/quit" {
            break;
        }
        drive_turn(&session, turn_input, &mut input, args.debug, &mut output).await?;
        output.flush()?;
    }
    Ok(())
}

async fn compose_session(
    config: &Config,
    agent_id: AgentId,
    initialize: bool,
) -> Result<Session, ReplError> {
    std::fs::create_dir_all(&config.stratum.storage_root)?;
    if initialize {
        std::fs::create_dir(config.stratum.storage_root.join(agent_id.to_string()))?;
    }
    let filesystem: Arc<dyn Filesystem> = Arc::new(LocalFilesystem::new(LocalFilesystemConfig {
        root: config.stratum.storage_root.clone(),
        max_file_bytes: None,
    })?);
    let store = Arc::new(FilesystemAgentStore::new(filesystem, agent_root(agent_id)?));

    if initialize {
        store
            .initialize(agent_id, DEFAULT_AGENT_NAME.to_owned())
            .await?;
    } else {
        store.load_agent().await?;
    }

    let store: Arc<dyn AgentStore> = store;
    let bus: Arc<dyn EventStreamBus> = Arc::new(StoreEventStreamBus::new(
        store.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
    ));
    let agent = build_default_agent(
        agent_id,
        store.clone(),
        bus.clone(),
        select_provider(config)?,
        approval_registry()?,
    )?;

    Ok(Session {
        agent_id,
        agent,
        store,
        bus,
        storage_root: config.stratum.storage_root.clone(),
    })
}

async fn restore_session<R: BufRead, W: Write>(
    session: &Session,
    input: &mut R,
    debug: bool,
    output: &mut W,
) -> Result<(), ReplError> {
    let state = session.store.load_agent().await?;
    if state.status == AgentStatus::Running {
        let mut events = session
            .bus
            .subscribe_agent(session.agent_id, ReplayStart::New)
            .await?;
        session.agent.resume().await?;
        consume_turn_events(session, &mut events, input, debug, output).await
    } else {
        session.agent.load_history().await?;
        Ok(())
    }
}

async fn drive_turn<R: BufRead, W: Write>(
    session: &Session,
    turn_input: &str,
    input: &mut R,
    debug: bool,
    output: &mut W,
) -> Result<(), ReplError> {
    let mut events = session
        .bus
        .subscribe_agent(session.agent_id, ReplayStart::New)
        .await?;
    session
        .agent
        .run_turn(ChatMessage::user(turn_input))
        .await?;
    consume_turn_events(session, &mut events, input, debug, output).await
}

async fn consume_turn_events<R: BufRead, W: Write>(
    session: &Session,
    events: &mut EventStream,
    input: &mut R,
    debug: bool,
    output: &mut W,
) -> Result<(), ReplError> {
    while let Some(record) = events.next().await {
        let record = record?;
        if debug {
            serde_json::to_writer(&mut *output, &record.envelope)?;
            output.write_all(b"\n")?;
        }
        let RuntimeEvent::Agent { event, .. } = &record.envelope.event else {
            continue;
        };
        match event {
            AgentEvent::ToolApprovalRequested {
                approval_id,
                tool_name,
                arguments,
                tool_kind,
                danger_level,
                ..
            } => {
                writeln!(
                    output,
                    "approval {approval_id}: tool {tool_name} ({tool_kind:?}, {danger_level:?})"
                )?;
                output.write_all(b"arguments: ")?;
                serde_json::to_writer(&mut *output, arguments)?;
                output.write_all(b"\n")?;
                resolve_approval(&session.agent, *approval_id, input, output).await?;
            }
            AgentEvent::Message { message, .. } if message.role == ChatRole::Assistant => {
                match &message.content {
                    ChatContent::Text(text) => output.write_all(text.as_bytes())?,
                    ChatContent::Json(value) => serde_json::to_writer(&mut *output, value)?,
                    _ => continue,
                }
                output.write_all(b"\n")?;
            }
            AgentEvent::Failed { error_text, .. } => {
                eprintln!("agent failed: {error_text}");
                return Ok(());
            }
            AgentEvent::Cancelled { .. } => {
                eprintln!("agent cancelled");
                return Ok(());
            }
            AgentEvent::Finished { .. } => return Ok(()),
            _ => {}
        }
    }
    Err(ReplError::EventStreamClosed)
}

async fn resolve_approval<R: BufRead, W: Write>(
    agent: &Agent,
    approval_id: ApprovalId,
    input: &mut R,
    output: &mut W,
) -> Result<(), ReplError> {
    loop {
        writeln!(output, "enter approve or reject")?;
        output.flush()?;
        let mut line = String::new();
        let bytes_read = input.read_line(&mut line)?;
        let decision = match line.trim() {
            "approve" => ApprovalDecision::Approve,
            "reject" => ApprovalDecision::Reject,
            _ if bytes_read == 0 => ApprovalDecision::Reject,
            _ => {
                writeln!(output, "enter approve or reject")?;
                continue;
            }
        };
        agent.resolve_tool_approval(approval_id, decision).await?;
        return Ok(());
    }
}

fn approval_registry() -> Result<Arc<dyn ToolRegistry>, ReplError> {
    let mut registry = BuiltinToolRegistry::new(ToolPermissionMode::RequireApproval);
    registry.register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)?;
    Ok(Arc::new(registry))
}

fn agent_root(agent_id: AgentId) -> Result<VirtualPath, ReplError> {
    VirtualPath::try_from(format!("/{agent_id}").as_str()).map_err(ReplError::from)
}

fn select_provider(config: &Config) -> Result<Arc<dyn LlmProvider>, ReplError> {
    let model = &config.stratum.model;
    match model.provider_name() {
        "openai" => {
            let provider = config
                .openai
                .as_ref()
                .ok_or(ReplError::MissingProviderConfiguration { provider: "openai" })?;
            Ok(Arc::new(OpenAICompatibleProvider::new(
                OPENAI_BASE_URL,
                ApiKey::new(provider.api_key.clone()),
                model.clone(),
            )))
        }
        "deepseek" => {
            let provider =
                config
                    .deepseek
                    .as_ref()
                    .ok_or(ReplError::MissingProviderConfiguration {
                        provider: "deepseek",
                    })?;
            let deepseek_model = match model.model_name() {
                "deepseek-v4-flash" => DeepSeekModel::V4Flash,
                "deepseek-v4-pro" => DeepSeekModel::V4Pro,
                _ => {
                    return Err(ReplError::UnsupportedModel {
                        model: model.clone(),
                    });
                }
            };
            Ok(Arc::new(DeepSeekProvider::new(
                DEEPSEEK_BASE_URL,
                ApiKey::new(provider.api_key.clone()),
                deepseek_model,
                DeepSeekThinking::Disabled,
            )))
        }
        provider => Err(ReplError::UnsupportedProvider {
            provider: provider.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Cursor, sync::Arc};

    use super::{
        Args, Config, ReplError, Session, agent_root, approval_registry, consume_turn_events,
        drive_turn, restore_session, select_provider,
    };
    use clap::Parser;
    use wyse_core::{
        AgentEvent, AgentId, CallId, ChatContent, ChatRole, DangerLevel, RuntimeEvent,
        StreamEnvelope, ToolCallDelta, ToolKind, ToolName,
    };
    use wyse_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig};
    use wyse_infra::{EventStream, EventStreamBus, event_stream_bus::InMemoryEventStreamBus};
    use wyse_llm::{ChatStreamEvent, FinishReason, MockLlmProvider};
    use wyse_store::{
        AgentState, AgentStatus, AgentStore, FilesystemAgentStore, StoreEventStreamBus,
    };

    async fn test_session(
        root: &std::path::Path,
        agent_id: AgentId,
        provider: MockLlmProvider,
        initialize: bool,
    ) -> Result<Session, ReplError> {
        if initialize {
            fs::create_dir(root.join(agent_id.to_string()))?;
        }
        let filesystem: Arc<dyn Filesystem> =
            Arc::new(LocalFilesystem::new(LocalFilesystemConfig {
                root: root.to_path_buf(),
                max_file_bytes: None,
            })?);
        let store = Arc::new(FilesystemAgentStore::new(filesystem, agent_root(agent_id)?));
        if initialize {
            store.initialize(agent_id, "test-agent".to_owned()).await?;
        }
        let store: Arc<dyn AgentStore> = store;
        let bus: Arc<dyn EventStreamBus> = Arc::new(StoreEventStreamBus::new(
            store.clone(),
            Arc::new(InMemoryEventStreamBus::default()),
        ));
        let agent = wyse_agent_builtin::build_default_agent(
            agent_id,
            store.clone(),
            bus.clone(),
            Arc::new(provider),
            approval_registry()?,
        )?;

        Ok(Session {
            agent_id,
            agent,
            store,
            bus,
            storage_root: root.to_path_buf(),
        })
    }

    #[test]
    fn approval_registry_registers_echo_as_low_danger_read_tool() -> Result<(), ReplError> {
        let registry = approval_registry()?;

        let specs = registry.specs();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name.as_str(), "echo");
        assert_eq!(
            registry.authorization(&ToolName::from("echo"))?,
            Some((ToolKind::Read, DangerLevel::Low))
        );

        Ok(())
    }

    fn mock_response(text: &str) -> MockLlmProvider {
        MockLlmProvider::new().with_stream_events(vec![
            ChatStreamEvent::TextDelta {
                delta: text.to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ])
    }

    fn approval_provider() -> MockLlmProvider {
        MockLlmProvider::new()
            .with_stream_events(vec![
                ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                    index: 0,
                    call_id: Some(CallId::from("echo-1")),
                    name: Some("echo".to_owned()),
                    arguments_delta: r#"{"message":"hello"}"#.to_owned(),
                }),
                ChatStreamEvent::Finished {
                    finish_reason: FinishReason::ToolCalls,
                    usage: None,
                },
            ])
            .with_stream_events(vec![
                ChatStreamEvent::TextDelta {
                    delta: "done".to_owned(),
                },
                ChatStreamEvent::Finished {
                    finish_reason: FinishReason::Stop,
                    usage: None,
                },
            ])
    }

    fn persisted_messages(
        root: &std::path::Path,
        agent_id: AgentId,
        last_seq: u64,
    ) -> Result<Vec<StreamEnvelope>, ReplError> {
        (1..=last_seq)
            .map(|seq| {
                Ok(serde_json::from_slice(&fs::read(
                    root.join(agent_id.to_string())
                        .join(format!("messages/{seq}.json")),
                )?)?)
            })
            .collect()
    }

    fn assistant_messages(envelopes: &[StreamEnvelope]) -> Vec<StreamEnvelope> {
        envelopes
            .iter()
            .filter(|envelope| {
                matches!(
                    &envelope.event,
                    RuntimeEvent::Agent {
                        event: AgentEvent::Message { message, .. },
                        ..
                    } if message.role == wyse_core::ChatRole::Assistant
                )
            })
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn approval_approves_echo_and_persists_the_tool_result() -> Result<(), ReplError> {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let session = test_session(&root, agent_id, approval_provider(), true).await?;
        let mut input = Cursor::new(b"approve\n");
        let mut output = Vec::new();

        drive_turn(&session, "use echo", &mut input, true, &mut output).await?;

        let output = String::from_utf8(output).expect("renderer writes UTF-8");
        assert!(output.contains("approval"));
        assert!(output.contains("done"));
        let envelopes = output
            .lines()
            .filter(|line| line.starts_with('{'))
            .map(serde_json::from_str::<StreamEnvelope>)
            .collect::<Result<Vec<_>, _>>()?;
        assert!(envelopes.iter().any(|envelope| matches!(
            envelope.event,
            RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalRequested { .. },
                ..
            }
        )));
        assert!(envelopes.iter().any(|envelope| matches!(
            envelope.event,
            RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalResolved {
                    decision: wyse_core::ApprovalDecision::Approve,
                    ..
                },
                ..
            }
        )));
        let persisted = persisted_messages(&root, agent_id, 4)?;
        assert!(matches!(
            &persisted[2].event,
            RuntimeEvent::Agent {
                agent_id: persisted_agent_id,
                event: AgentEvent::Message { message, .. },
            } if *persisted_agent_id == agent_id
                && message.role == ChatRole::Tool
                && message.content == ChatContent::Json(serde_json::json!({ "message": "hello" }))
                && message.tool_call_id == Some(CallId::from("echo-1"))
        ));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn approval_rejects_echo_and_persists_the_runtime_rejection() -> Result<(), ReplError> {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let session = test_session(&root, agent_id, approval_provider(), true).await?;
        let mut input = Cursor::new(b"reject\n");
        let mut output = Vec::new();

        drive_turn(&session, "use echo", &mut input, false, &mut output).await?;

        let persisted = persisted_messages(&root, agent_id, 4)?;
        assert!(matches!(
            &persisted[2].event,
            RuntimeEvent::Agent {
                agent_id: persisted_agent_id,
                event: AgentEvent::Message { message, .. },
            } if *persisted_agent_id == agent_id
                && message.role == ChatRole::Tool
                && message.content == ChatContent::Json(serde_json::json!({
                    "error": {
                        "type": "approval_rejected",
                        "message": "user rejected tool call"
                    }
                }))
                && message.tool_call_id == Some(CallId::from("echo-1"))
        ));
        assert!(matches!(
            &persisted[3].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Message { message, .. },
                ..
            } if message.role == ChatRole::Assistant && message.content == ChatContent::Text("done".to_owned())
        ));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn approval_rejects_echo_when_input_reaches_eof() -> Result<(), ReplError> {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let session = test_session(&root, agent_id, approval_provider(), true).await?;
        let mut input = Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();

        drive_turn(&session, "use echo", &mut input, false, &mut output).await?;

        let persisted = persisted_messages(&root, agent_id, 4)?;
        assert!(matches!(
            &persisted[2].event,
            RuntimeEvent::Agent {
                agent_id: persisted_agent_id,
                event: AgentEvent::Message { message, .. },
            } if *persisted_agent_id == agent_id
                && message.role == ChatRole::Tool
                && message.content == ChatContent::Json(serde_json::json!({
                    "error": {
                        "type": "approval_rejected",
                        "message": "user rejected tool call"
                    }
                }))
                && message.tool_call_id == Some(CallId::from("echo-1"))
        ));
        assert!(matches!(
            &persisted[3].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Message { message, .. },
                ..
            } if message.role == ChatRole::Assistant && message.content == ChatContent::Text("done".to_owned())
        ));

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn drive_turn_renders_bus_events_and_matches_persisted_messages() -> Result<(), ReplError>
    {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let provider = MockLlmProvider::new()
            .with_stream_events(vec![
                ChatStreamEvent::TextDelta {
                    delta: "first response".to_owned(),
                },
                ChatStreamEvent::Finished {
                    finish_reason: FinishReason::Stop,
                    usage: None,
                },
            ])
            .with_stream_events(vec![
                ChatStreamEvent::TextDelta {
                    delta: "second response".to_owned(),
                },
                ChatStreamEvent::Finished {
                    finish_reason: FinishReason::Stop,
                    usage: None,
                },
            ]);
        let session = test_session(&root, agent_id, provider, true).await?;
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        drive_turn(&session, "first input", &mut input, false, &mut output).await?;
        drive_turn(&session, "second input", &mut input, false, &mut output).await?;

        let output = String::from_utf8(output).expect("renderer writes UTF-8");
        assert!(output.contains("first response"));
        assert!(output.contains("second response"));
        assert!(!output.lines().any(|line| line.starts_with('{')));

        let state: AgentState = serde_json::from_slice(&fs::read(
            root.join(agent_id.to_string()).join("agent.json"),
        )?)?;
        assert_eq!(state.last_seq, 4);
        let persisted = (1..=4)
            .map(|seq| -> Result<StreamEnvelope, ReplError> {
                Ok(serde_json::from_slice(&fs::read(
                    root.join(agent_id.to_string())
                        .join(format!("messages/{seq}.json")),
                )?)?)
            })
            .collect::<Result<Vec<_>, ReplError>>()?;
        assert_eq!(
            persisted
                .iter()
                .map(StreamEnvelope::business_seq)
                .collect::<Vec<_>>(),
            vec![Some(1), Some(2), Some(3), Some(4)]
        );

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn drive_turn_debug_emits_assistant_envelope_from_bus() -> Result<(), ReplError> {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let session = test_session(&root, agent_id, mock_response("debug response"), true).await?;
        let mut output = Vec::new();
        let mut input = Cursor::new(Vec::<u8>::new());

        drive_turn(&session, "debug input", &mut input, true, &mut output).await?;

        let envelopes = String::from_utf8(output)
            .expect("debug renderer writes UTF-8")
            .lines()
            .filter(|line| line.starts_with('{'))
            .map(serde_json::from_str::<StreamEnvelope>)
            .collect::<Result<Vec<_>, _>>()?;
        let persisted: Vec<StreamEnvelope> = (1..=2)
            .map(|seq| -> Result<StreamEnvelope, ReplError> {
                Ok(serde_json::from_slice(&fs::read(
                    root.join(agent_id.to_string())
                        .join(format!("messages/{seq}.json")),
                )?)?)
            })
            .collect::<Result<_, ReplError>>()?;
        assert_eq!(
            assistant_messages(&envelopes),
            assistant_messages(&persisted)
        );

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn restore_session_loads_finished_history_and_advances_existing_store()
    -> Result<(), ReplError> {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let first = test_session(&root, agent_id, mock_response("first response"), true).await?;
        let mut input = Cursor::new(Vec::<u8>::new());
        drive_turn(&first, "first input", &mut input, false, &mut Vec::new()).await?;

        let restored =
            test_session(&root, agent_id, mock_response("resumed response"), false).await?;
        restore_session(&restored, &mut input, false, &mut Vec::new()).await?;
        drive_turn(
            &restored,
            "second input",
            &mut input,
            false,
            &mut Vec::new(),
        )
        .await?;

        let state: AgentState = serde_json::from_slice(&fs::read(
            root.join(agent_id.to_string()).join("agent.json"),
        )?)?;
        assert_eq!(state.last_seq, 4);
        assert!(
            root.join(agent_id.to_string())
                .join("messages/1.json")
                .is_file()
        );
        assert!(
            root.join(agent_id.to_string())
                .join("messages/4.json")
                .is_file()
        );

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn restore_session_resumes_running_turn_to_terminal_completion() -> Result<(), ReplError>
    {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let initial = test_session(&root, agent_id, MockLlmProvider::new(), true).await?;
        let mut input = Cursor::new(Vec::<u8>::new());
        drive_turn(
            &initial,
            "interrupted input",
            &mut input,
            false,
            &mut Vec::new(),
        )
        .await?;
        let state = initial.store.load_agent().await?;
        assert_eq!(state.status, AgentStatus::Failed);
        assert_eq!(state.last_seq, 1);
        initial
            .store
            .update_state(
                AgentStatus::Running,
                state.run_id,
                state.turn_id,
                state.usage,
            )
            .await?;
        assert_eq!(
            initial.store.load_agent().await?.status,
            AgentStatus::Running
        );

        let restored =
            test_session(&root, agent_id, mock_response("resumed response"), false).await?;
        let mut output = Vec::new();
        restore_session(&restored, &mut input, false, &mut output).await?;

        assert!(
            String::from_utf8(output)
                .expect("renderer writes UTF-8")
                .contains("resumed response")
        );
        let state = restored.store.load_agent().await?;
        assert_eq!(state.status, AgentStatus::Finished);
        assert_eq!(state.last_seq, 2);

        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[tokio::test]
    async fn consume_turn_events_rejects_stream_closure_without_terminal_event()
    -> Result<(), ReplError> {
        let agent_id = AgentId::new();
        let root = std::env::temp_dir().join(agent_id.to_string());
        fs::create_dir(&root)?;
        let session = test_session(&root, agent_id, MockLlmProvider::new(), true).await?;
        let mut events: EventStream = Box::pin(futures_util::stream::empty());
        let mut input = Cursor::new(Vec::<u8>::new());
        let error = consume_turn_events(&session, &mut events, &mut input, false, &mut Vec::new())
            .await
            .expect_err("stream closure before terminal event must fail");

        assert_eq!(
            error.to_string(),
            "event stream closed before a terminal agent event"
        );
        let _ = fs::remove_dir_all(root);
        Ok(())
    }

    #[test]
    fn parses_resume_and_debug_arguments() -> Result<(), ReplError> {
        let agent_id = AgentId::new();

        let args =
            Args::try_parse_from(["stratum-repl", "--resume", &agent_id.to_string(), "--debug"])?;

        assert_eq!(args.resume, Some(agent_id));
        assert!(args.debug);
        Ok(())
    }

    #[test]
    fn accepts_minimal_stratum_and_openai_configuration() -> Result<(), ReplError> {
        let config: Config = toml::from_str(
            r#"
[stratum]
storage_root = "./.stratum/repl"
model = "openai:gpt-4.1-mini"

[openai]
api_key = "test-key"
"#,
        )?;

        assert_eq!(config.stratum.model.as_str(), "openai:gpt-4.1-mini");
        Ok(())
    }

    #[test]
    fn rejects_unknown_stratum_configuration() {
        let result = toml::from_str::<Config>(
            r#"
[stratum]
storage_root = "./.stratum/repl"
model = "openai:gpt-4.1-mini"
unexpected = true

[openai]
api_key = "test-key"
"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn rejects_unsupported_provider_without_network_access() -> Result<(), ReplError> {
        let config: Config = toml::from_str(
            r#"
[stratum]
storage_root = "./.stratum/repl"
model = "custom:model"
"#,
        )?;

        match select_provider(&config) {
            Err(ReplError::UnsupportedProvider { .. }) => {}
            Err(error) => panic!("unexpected provider error: {error}"),
            Ok(_) => panic!("custom providers are unsupported"),
        }
        Ok(())
    }

    #[test]
    fn scopes_agent_store_to_agent_root() -> Result<(), ReplError> {
        let agent_id = AgentId::new();

        assert_eq!(agent_root(agent_id)?.as_str(), format!("/{agent_id}"));
        Ok(())
    }
}
