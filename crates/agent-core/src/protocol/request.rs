//! Requests: what a client asks of the daemon.

use std::collections::HashSet;
use std::num::NonZeroU32;

use serde::{Deserialize, Serialize};

use super::{EncodeError, MAX_REQUEST_BYTES, ProtocolError, ProtocolVersion, decode, encode};
use crate::config::{AbsolutePath, RepoName, TailscaleLogin};
use crate::ids::{AgentName, TaskId, TeamId};
use crate::questions::Verdict;
use crate::state::{MergeMode, TaskListText};

/// Most agents one `run` may name.
pub const MAX_RUN_AGENTS: usize = 16;

/// Most answers one `answer` may carry: the questions of one file.
pub const MAX_ANSWERS: usize = 256;

/// Longest text of a `say`, in bytes.
pub const MAX_SAY_BYTES: usize = 64 * 1024;

/// A request line: the client's number for it, who is asking and what for.
///
/// The daemon answers with a [`Response`](super::Response) of the same [`RequestId`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// The protocol version, always [`PROTOCOL_VERSION`](super::PROTOCOL_VERSION).
    protocol: ProtocolVersion,
    /// The client's number for this request, echoed in the response.
    pub id: RequestId,
    /// Who is asking: the activity of a control names them.
    pub client: Client,
    /// What is asked.
    #[serde(rename = "request")]
    pub command: Command,
}

/// A client's number for a request: any number, echoed in the response so that a client can match them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(pub u64);

/// Who sends a request.
///
/// Reading refuses fields that the variant does not have, `cli` included (see `ClientWire`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "ClientWire", tag = "kind", rename_all = "snake_case")]
pub enum Client {
    /// The `agent` command line: the human at the terminal.
    Cli,
    /// The dashboard, for the human who logged in.
    Dashboard {
        /// The Tailscale login the dashboard saw.
        login: TailscaleLogin,
    },
}

/// A [`Client`] as it is read. serde ignores the fields beside the tag of an internally tagged unit variant, so `cli`
/// is read as a variant without fields, which refuses them like every other variant.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum ClientWire {
    /// See [`Client::Cli`].
    Cli {},
    /// See [`Client::Dashboard`].
    Dashboard {
        /// See [`Client::Dashboard`].
        login: TailscaleLogin,
    },
}

impl From<ClientWire> for Client {
    fn from(wire: ClientWire) -> Self {
        match wire {
            ClientWire::Cli {} => Self::Cli,
            ClientWire::Dashboard { login } => Self::Dashboard { login },
        }
    }
}

/// What a client asks of the daemon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Command {
    /// Start a task or a team.
    Run(RunRequest),
    /// List the teams, or one team, in short.
    Status(StatusRequest),
    /// Show a team or a task in full.
    Show(ShowRequest),
    /// Answer a task's questions.
    Answer(AnswerRequest),
    /// Stop a task or a team.
    Stop(ControlRequest),
    /// Pause a task or a team.
    Pause(ControlRequest),
    /// Let a paused task or team go on.
    Unpause(ControlRequest),
    /// Start a stopped, failed or waiting task or team again.
    Resume(ControlRequest),
    /// Say something to a task or a team.
    Say(SayRequest),
    /// Run the repository's gate for a task.
    Gate(GateRequest),
    /// Receive the events of the state until the connection closes.
    Subscribe(SubscribeRequest),
}

/// The task or the team a command is about.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// A team and all its tasks.
    Team(TeamId),
    /// One task.
    Task(TaskId),
}

/// `run`: start a task or a team on a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunRequest {
    /// The repository, as `[repos.NAME]` of the configuration.
    pub repo: RepoName,
    /// The local checkout the client is in: the worktrees are made from it.
    pub checkout: AbsolutePath,
    /// The agents and investigators to run: one is a task alone, more are a team.
    pub agents: AgentList,
    /// The task list, as the client read it (inline text or the content of `-f`).
    pub task_list: TaskListText,
    /// The coordinator the client asks for (`--coordinator`), which wins over the task list's `Coordinator:` line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinator: Option<AgentName>,
    /// What happens to an approved pull request (`--no-merge`).
    pub merge: MergeMode,
}

/// `status`: the teams in short.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusRequest {
    /// The team or task to report on; every team when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Target>,
}

/// `show`: a team or a task in full.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShowRequest {
    /// What to show.
    pub target: Target,
}

/// `answer`: the human's answers to one round of a task's questions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerRequest {
    /// The task that asked.
    pub task: TaskId,
    /// The round of questions that is answered.
    pub round: NonZeroU32,
    /// A verdict for each question, which the daemon checks against the questions file.
    pub answers: AnswerList,
}

/// `stop`, `pause`, `unpause` and `resume`: a control of a task or a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    /// What to control.
    pub target: Target,
}

/// `say`: a message from the human to a task or a team.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SayRequest {
    /// Who hears it.
    pub target: Target,
    /// What is said.
    pub text: SayText,
}

/// `gate`: run the repository's gate in a task's worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateRequest {
    /// The task whose worktree the gate runs in.
    pub task: TaskId,
}

/// `subscribe`: receive events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRequest {
    /// Only the events of this team; every team's when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team: Option<TeamId>,
}

/// Implements borrowing and checked `Vec` conversion for a nonempty, bounded list with unique keys.
macro_rules! checked_list {
    ($ty:ident, $entry:ty, $error:ident, $max:expr, $field:ident, $key:expr) => {
        impl $ty {
            /// The entries, in the order given.
            #[must_use]
            pub fn as_slice(&self) -> &[$entry] {
                &self.0
            }
        }

        impl TryFrom<Vec<$entry>> for $ty {
            type Error = $error;

            fn try_from(value: Vec<$entry>) -> Result<Self, Self::Error> {
                if value.is_empty() {
                    return Err($error::Empty);
                }
                if value.len() > $max {
                    return Err($error::TooMany {
                        count: value.len(),
                        max: $max,
                    });
                }
                let mut seen = HashSet::new();
                for entry in &value {
                    let key = $key(entry);
                    if !seen.insert(key.clone()) {
                        return Err($error::Duplicate { $field: key });
                    }
                }
                Ok(Self(value))
            }
        }

        impl From<$ty> for Vec<$entry> {
            fn from(list: $ty) -> Self {
                list.0
            }
        }
    };
}

/// The agents of a `run`: at least one, at most [`MAX_RUN_AGENTS`], none twice, in the order given.
///
/// Whether each is configured, and as which tier, is for the daemon to decide: the names have one form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<AgentName>", into = "Vec<AgentName>")]
pub struct AgentList(Vec<AgentName>);

/// Why a list of agents is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AgentListError {
    /// The list is empty.
    #[error("name at least one agent")]
    Empty,
    /// The list is longer than [`MAX_RUN_AGENTS`].
    #[error("{count} agents named; the most is {max}")]
    TooMany {
        /// How many were named.
        count: usize,
        /// The limit.
        max: usize,
    },
    /// An agent is named twice.
    #[error("{name} is named twice")]
    Duplicate {
        /// The repeated name.
        name: AgentName,
    },
}

checked_list!(
    AgentList,
    AgentName,
    AgentListError,
    MAX_RUN_AGENTS,
    name,
    |name: &AgentName| (*name).clone()
);

/// The verdict the human gave one question of a round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnswerInput {
    /// The question's number in the file.
    pub question: NonZeroU32,
    /// The verdict: `ok` or an own answer.
    pub verdict: Verdict,
}

/// The answers of an `answer`: at least one, at most [`MAX_ANSWERS`], none for a question twice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<AnswerInput>", into = "Vec<AnswerInput>")]
pub struct AnswerList(Vec<AnswerInput>);

/// Why a list of answers is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerListError {
    /// The list is empty.
    #[error("give at least one answer")]
    Empty,
    /// The list is longer than [`MAX_ANSWERS`].
    #[error("{count} answers given; the most is {max}")]
    TooMany {
        /// How many were given.
        count: usize,
        /// The limit.
        max: usize,
    },
    /// Two answers are for one question.
    #[error("question {question} is answered twice")]
    Duplicate {
        /// The repeated question.
        question: NonZeroU32,
    },
}

checked_list!(
    AnswerList,
    AnswerInput,
    AnswerListError,
    MAX_ANSWERS,
    question,
    |answer: &AnswerInput| answer.question
);

/// The text of a `say`: not blank, at most [`MAX_SAY_BYTES`], without NUL. Lines are fine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SayText(String);

/// Why a text is refused as a `say`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SayError {
    /// The text is empty or only whitespace.
    #[error("there is nothing to say")]
    Blank,
    /// The text is longer than [`MAX_SAY_BYTES`].
    #[error("the text has {len} bytes; the limit is {max}")]
    TooLong {
        /// The text's length, in bytes.
        len: usize,
        /// The limit.
        max: usize,
    },
    /// The text contains NUL.
    #[error("the text contains a NUL character")]
    ContainsNul,
}

impl SayText {
    /// The text, as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SayText {
    type Error = SayError;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.trim().is_empty() {
            return Err(SayError::Blank);
        }
        if text.len() > MAX_SAY_BYTES {
            return Err(SayError::TooLong {
                len: text.len(),
                max: MAX_SAY_BYTES,
            });
        }
        if text.contains('\0') {
            return Err(SayError::ContainsNul);
        }
        Ok(Self(text))
    }
}

impl From<SayText> for String {
    fn from(text: SayText) -> Self {
        text.0
    }
}

impl Request {
    /// A request.
    #[must_use]
    pub const fn new(id: RequestId, client: Client, command: Command) -> Self {
        Self {
            protocol: ProtocolVersion,
            id,
            client,
            command,
        }
    }

    /// The request as one line ending with a newline.
    ///
    /// # Errors
    /// [`EncodeError`] when the serializer fails.
    pub fn to_line(&self) -> Result<String, EncodeError> {
        encode(self)
    }

    /// Reads a line a client sent.
    ///
    /// # Errors
    /// [`ProtocolError`]: the line is over [`MAX_REQUEST_BYTES`](super::MAX_REQUEST_BYTES), declares another protocol
    /// version (checked first, so that a client of another version is told so whatever else it sent), or is not a
    /// valid request.
    pub fn from_line(line: &str) -> Result<Self, ProtocolError> {
        decode(line, MAX_REQUEST_BYTES)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::state::fixtures::{LIST, assert_refused, list, nz, task, team_id};

    /// An agent name.
    fn agent(name: &str) -> AgentName {
        name.parse().unwrap()
    }

    /// An answer to question `question`.
    fn answer(question: u32, verdict: &str) -> AnswerInput {
        AnswerInput {
            question: nz(question),
            verdict: Verdict::from_text(verdict).unwrap().unwrap(),
        }
    }

    /// The request of `command` as the JSON of its body.
    fn body(command: &Command) -> serde_json::Value {
        serde_json::to_value(command).unwrap()
    }

    // ---- the validated parts ------------------------------------------------------------------------------------

    #[test]
    fn a_list_of_agents_keeps_the_order_given() {
        let list = AgentList::try_from(vec![agent("inv-opus"), agent("codex-1"), agent("claude-1")]).unwrap();

        let names: Vec<&str> = list.as_slice().iter().map(AgentName::as_str).collect();

        assert_eq!(names, ["inv-opus", "codex-1", "claude-1"]);
        assert_eq!(
            serde_json::to_value(&list).unwrap(),
            json!(["inv-opus", "codex-1", "claude-1"])
        );
        assert_eq!(Vec::<AgentName>::from(list.clone()).len(), 3);
    }

    #[test]
    fn a_list_of_agents_needs_one_to_sixteen_names_none_twice() {
        assert_eq!(AgentList::try_from(vec![]), Err(AgentListError::Empty));
        let many: Vec<AgentName> = (0..=MAX_RUN_AGENTS).map(|n| agent(&format!("agent-{n}"))).collect();
        assert_eq!(
            AgentList::try_from(many.clone()),
            Err(AgentListError::TooMany {
                count: MAX_RUN_AGENTS + 1,
                max: MAX_RUN_AGENTS
            })
        );
        AgentList::try_from(many.get(..MAX_RUN_AGENTS).unwrap().to_vec()).unwrap();
        assert_eq!(
            AgentList::try_from(vec![agent("a"), agent("b"), agent("a")]),
            Err(AgentListError::Duplicate { name: agent("a") })
        );
    }

    #[test]
    fn a_list_of_agents_is_checked_when_read() {
        assert_refused::<AgentList>(&json!([]));
        assert_refused::<AgentList>(&json!(["a", "a"]));
        assert_refused::<AgentList>(&json!(["Not Valid"]));
        assert_refused::<AgentList>(&json!("codex-1"));
        serde_json::from_value::<AgentList>(json!(["codex-1"])).unwrap();
    }

    #[test]
    fn a_list_of_answers_needs_one_to_256_none_for_a_question_twice() {
        assert_eq!(AnswerList::try_from(vec![]), Err(AnswerListError::Empty));
        let many: Vec<AnswerInput> = (1..=u32::try_from(MAX_ANSWERS).unwrap() + 1)
            .map(|n| answer(n, "ok"))
            .collect();
        assert_eq!(
            AnswerList::try_from(many.clone()),
            Err(AnswerListError::TooMany {
                count: MAX_ANSWERS + 1,
                max: MAX_ANSWERS
            })
        );
        AnswerList::try_from(many.get(..MAX_ANSWERS).unwrap().to_vec()).unwrap();
        assert_eq!(
            AnswerList::try_from(vec![answer(1, "ok"), answer(2, "no"), answer(1, "yes")]),
            Err(AnswerListError::Duplicate { question: nz(1) })
        );
        let in_order = AnswerList::try_from(vec![answer(2, "ok"), answer(1, "x")]).unwrap();
        assert_eq!(
            in_order.as_slice().iter().map(|a| a.question.get()).collect::<Vec<_>>(),
            [2, 1]
        );
        assert_eq!(Vec::<AnswerInput>::from(in_order).len(), 2);
    }

    #[test]
    fn answers_are_written_as_question_numbers_with_the_verdict_text() {
        let answers = AnswerList::try_from(vec![answer(1, "OK"), answer(2, "b) on disk")]).unwrap();

        let value = serde_json::to_value(&answers).unwrap();

        assert_eq!(
            value,
            json!([{"question": 1, "verdict": "ok"}, {"question": 2, "verdict": "b) on disk"}])
        );
        assert_eq!(serde_json::from_value::<AnswerList>(value).unwrap(), answers);
        assert_refused::<AnswerList>(&json!([{"question": 0, "verdict": "ok"}]));
        assert_refused::<AnswerList>(&json!([{"question": 1, "verdict": ""}]));
        assert_refused::<AnswerList>(&json!([{"question": 1}]));
        assert_refused::<AnswerList>(&json!([{"question": 1, "verdict": "ok", "extra": 1}]));
    }

    #[test]
    fn a_say_text_is_any_non_blank_text_up_to_64_kib_without_nul() {
        for text in ["hi", "  padded  ", "two\nlines", "Żółć", &"x".repeat(MAX_SAY_BYTES)] {
            let said = SayText::try_from(text.to_owned()).unwrap();

            assert_eq!(said.as_str(), text);
            assert_eq!(String::from(said), text);
        }
        for blank in ["", " ", "\n\t "] {
            assert_eq!(SayText::try_from(blank.to_owned()), Err(SayError::Blank), "{blank:?}");
        }
        let long = "x".repeat(MAX_SAY_BYTES + 1);
        assert_eq!(
            SayText::try_from(long),
            Err(SayError::TooLong {
                len: MAX_SAY_BYTES + 1,
                max: MAX_SAY_BYTES
            })
        );
        assert_eq!(SayText::try_from(String::from("a\0b")), Err(SayError::ContainsNul));
        assert_refused::<SayText>(&json!(""));
        assert_refused::<SayText>(&json!(5));
        assert_eq!(
            serde_json::to_value(SayText::try_from(String::from("hi")).unwrap()).unwrap(),
            json!("hi")
        );
    }

    // ---- the commands as JSON -----------------------------------------------------------------------------------

    /// The sample task and team of these tests, as the text of their IDs.
    const TASK: &str = "task-20261111-153002-k3x9";
    const TEAM: &str = "team-20261111-153001-t5m8";

    #[test]
    fn every_command_has_its_json_and_reads_back_equal() {
        let run = RunRequest {
            repo: "team-agents".parse().unwrap(),
            checkout: "/home/dev/projects/team-agents".parse().unwrap(),
            agents: AgentList::try_from(vec![agent("codex-1"), agent("inv-opus")]).unwrap(),
            task_list: list(),
            coordinator: None,
            merge: MergeMode::Autopilot,
        };
        let with_coordinator = RunRequest {
            coordinator: Some(agent("inv-opus")),
            merge: MergeMode::NoMerge,
            ..run.clone()
        };
        let cases: Vec<(Command, serde_json::Value)> = vec![
            (
                Command::Run(run),
                json!({"command": "run", "repo": "team-agents", "checkout": "/home/dev/projects/team-agents",
                    "agents": ["codex-1", "inv-opus"], "task_list": LIST, "merge": "autopilot"}),
            ),
            (
                Command::Run(with_coordinator),
                json!({"command": "run", "repo": "team-agents", "checkout": "/home/dev/projects/team-agents",
                    "agents": ["codex-1", "inv-opus"], "task_list": LIST, "coordinator": "inv-opus", "merge": "no_merge"}),
            ),
            (
                Command::Status(StatusRequest { target: None }),
                json!({"command": "status"}),
            ),
            (
                Command::Status(StatusRequest {
                    target: Some(Target::Team(team_id(1))),
                }),
                json!({"command": "status", "target": {"team": TEAM}}),
            ),
            (
                Command::Show(ShowRequest {
                    target: Target::Task(task(2)),
                }),
                json!({"command": "show", "target": {"task": TASK}}),
            ),
            (
                Command::Answer(AnswerRequest {
                    task: task(2),
                    round: nz(1),
                    answers: AnswerList::try_from(vec![answer(1, "ok"), answer(2, "mine")]).unwrap(),
                }),
                json!({"command": "answer", "task": TASK, "round": 1,
                    "answers": [{"question": 1, "verdict": "ok"}, {"question": 2, "verdict": "mine"}]}),
            ),
            (
                Command::Stop(ControlRequest {
                    target: Target::Task(task(2)),
                }),
                json!({"command": "stop", "target": {"task": TASK}}),
            ),
            (
                Command::Pause(ControlRequest {
                    target: Target::Team(team_id(1)),
                }),
                json!({"command": "pause", "target": {"team": TEAM}}),
            ),
            (
                Command::Unpause(ControlRequest {
                    target: Target::Team(team_id(1)),
                }),
                json!({"command": "unpause", "target": {"team": TEAM}}),
            ),
            (
                Command::Resume(ControlRequest {
                    target: Target::Task(task(2)),
                }),
                json!({"command": "resume", "target": {"task": TASK}}),
            ),
            (
                Command::Say(SayRequest {
                    target: Target::Team(team_id(1)),
                    text: SayText::try_from(String::from("hello")).unwrap(),
                }),
                json!({"command": "say", "target": {"team": TEAM}, "text": "hello"}),
            ),
            (
                Command::Gate(GateRequest { task: task(2) }),
                json!({"command": "gate", "task": TASK}),
            ),
            (
                Command::Subscribe(SubscribeRequest { team: None }),
                json!({"command": "subscribe"}),
            ),
            (
                Command::Subscribe(SubscribeRequest { team: Some(team_id(1)) }),
                json!({"command": "subscribe", "team": TEAM}),
            ),
        ];
        for (command, expected) in cases {
            let value = body(&command);

            assert_eq!(value, expected);
            assert_eq!(serde_json::from_value::<Command>(value).unwrap(), command);
        }
    }

    #[test]
    fn a_target_is_a_team_or_a_task_by_the_kind_of_its_id() {
        assert_eq!(
            serde_json::to_value(Target::Team(team_id(1))).unwrap(),
            json!({"team": TEAM})
        );
        assert_eq!(
            serde_json::to_value(Target::Task(task(2))).unwrap(),
            json!({"task": TASK})
        );
        assert_refused::<Target>(&json!({"team": TASK}));
        assert_refused::<Target>(&json!({"task": TEAM}));
        assert_refused::<Target>(&json!({"team": TEAM, "task": TASK}));
        assert_refused::<Target>(&json!({}));
        assert_refused::<Target>(&json!(TEAM));
    }

    #[test]
    fn a_run_request_with_a_bad_part_is_refused() {
        let good = json!({"command": "run", "repo": "team-agents", "checkout": "/home/u/r",
            "agents": ["codex-1"], "task_list": LIST, "merge": "autopilot"});
        serde_json::from_value::<Command>(good.clone()).unwrap();
        for (key, bad) in [
            ("repo", json!("a/b")),
            ("checkout", json!("relative/path")),
            ("agents", json!([])),
            ("task_list", json!("1. F\n[ ] 1.x Bad\n")),
            ("task_list", json!("")),
            ("coordinator", json!("Not Valid")),
            ("merge", json!("whenever")),
            ("extra", json!(1)),
        ] {
            let mut value = good.clone();
            value[key] = bad;

            assert_refused::<Command>(&value);
        }
        for key in ["repo", "checkout", "agents", "task_list", "merge"] {
            let mut value = good.clone();
            value.as_object_mut().unwrap().remove(key);

            assert_refused::<Command>(&value);
        }
    }

    #[test]
    fn a_command_with_a_zero_round_a_missing_part_or_an_unknown_field_is_refused() {
        for value in [
            json!({"command": "answer", "task": TASK, "round": 0, "answers": [{"question": 1, "verdict": "ok"}]}),
            json!({"command": "answer", "task": TASK, "answers": [{"question": 1, "verdict": "ok"}]}),
            json!({"command": "answer", "task": TASK, "round": 1}),
            json!({"command": "say", "target": {"team": TEAM}, "text": " "}),
            json!({"command": "say", "target": {"team": TEAM}}),
            json!({"command": "gate"}),
            json!({"command": "gate", "task": TEAM}),
            json!({"command": "subscribe", "team": TASK}),
            json!({"command": "subscribe", "extra": true}),
            json!({"command": "show"}),
        ] {
            assert_refused::<Command>(&value);
        }
    }

    // ---- who is asking ------------------------------------------------------------------------------------------

    #[test]
    fn a_client_is_the_cli_or_the_dashboard_with_the_login_it_saw() {
        let dashboard = Client::Dashboard {
            login: "You@Example.com".parse().unwrap(),
        };

        assert_eq!(serde_json::to_value(Client::Cli).unwrap(), json!({"kind": "cli"}));
        assert_eq!(
            serde_json::to_value(&dashboard).unwrap(),
            json!({"kind": "dashboard", "login": "you@example.com"})
        );
        assert_eq!(
            serde_json::from_value::<Client>(json!({"kind": "cli"})).unwrap(),
            Client::Cli
        );
        assert_refused::<Client>(&json!({"kind": "dashboard"}));
        assert_refused::<Client>(&json!({"kind": "dashboard", "login": "no-at-sign"}));
        assert_refused::<Client>(&json!({"kind": "script"}));
    }

    #[test]
    fn a_request_id_is_any_number_written_as_a_number() {
        assert_eq!(serde_json::to_value(RequestId(0)).unwrap(), json!(0));
        assert_eq!(serde_json::to_value(RequestId(u64::MAX)).unwrap(), json!(u64::MAX));
        assert_eq!(serde_json::from_value::<RequestId>(json!(42)).unwrap(), RequestId(42));
        assert_refused::<RequestId>(&json!(-1));
        assert_refused::<RequestId>(&json!("1"));
    }
}
