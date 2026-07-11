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
    AgentEvent, AgentId, ChatContent, ChatMessage, ChatRole, ModelId, ModelIdParseError,
    ReplayStart, RuntimeEvent,
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
    restore_session(&session, args.debug, &mut output).await?;

    let stdin = std::io::stdin();
    let mut input = stdin.lock();
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
        let input = line.trim_end_matches(['\r', '\n']);
        if input == "/quit" {
            break;
        }
        drive_turn(&session, input, args.debug, &mut output).await?;
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
    )?;

    Ok(Session {
        agent_id,
        agent,
        store,
        bus,
        storage_root: config.stratum.storage_root.clone(),
    })
}

async fn restore_session<W: Write>(
    session: &Session,
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
        consume_turn_events(&mut events, debug, output).await
    } else {
        session.agent.load_history().await?;
        Ok(())
    }
}

async fn drive_turn<W: Write>(
    session: &Session,
    input: &str,
    debug: bool,
    output: &mut W,
) -> Result<(), ReplError> {
    let mut events = session
        .bus
        .subscribe_agent(session.agent_id, ReplayStart::New)
        .await?;
    session.agent.run_turn(ChatMessage::user(input)).await?;
    consume_turn_events(&mut events, debug, output).await
}

async fn consume_turn_events<W: Write>(
    events: &mut EventStream,
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
    use std::{fs, sync::Arc};

    use clap::Parser;
    use wyse_core::{AgentEvent, AgentId, RuntimeEvent, StreamEnvelope};
    use wyse_filesystem::{Filesystem, LocalFilesystem, LocalFilesystemConfig};
    use wyse_infra::{EventStream, EventStreamBus, event_stream_bus::InMemoryEventStreamBus};
    use wyse_llm::{ChatStreamEvent, FinishReason, MockLlmProvider};
    use wyse_store::{
        AgentState, AgentStatus, AgentStore, FilesystemAgentStore, StoreEventStreamBus,
    };

    use super::{
        Args, Config, ReplError, Session, agent_root, consume_turn_events, drive_turn,
        restore_session, select_provider,
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
        )?;

        Ok(Session {
            agent_id,
            agent,
            store,
            bus,
            storage_root: root.to_path_buf(),
        })
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

        drive_turn(&session, "first input", false, &mut output).await?;
        drive_turn(&session, "second input", false, &mut output).await?;

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

        drive_turn(&session, "debug input", true, &mut output).await?;

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
        drive_turn(&first, "first input", false, &mut Vec::new()).await?;

        let restored =
            test_session(&root, agent_id, mock_response("resumed response"), false).await?;
        restore_session(&restored, false, &mut Vec::new()).await?;
        drive_turn(&restored, "second input", false, &mut Vec::new()).await?;

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
        drive_turn(&initial, "interrupted input", false, &mut Vec::new()).await?;
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
        restore_session(&restored, false, &mut output).await?;

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
        let mut events: EventStream = Box::pin(futures_util::stream::empty());
        let error = consume_turn_events(&mut events, false, &mut Vec::new())
            .await
            .expect_err("stream closure before terminal event must fail");

        assert_eq!(
            error.to_string(),
            "event stream closed before a terminal agent event"
        );
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
