//! The model and effort each subagent of a turn runs at, and how the agent is told to choose
//! them. And the calls that started subagents and workflows, which a subagent that runs
//! again shows running again.

use super::install;
use super::models::{manifest, parse_effort};
use crate::truncate_json;
use hoshi_core::{Effort, ProviderOutput, SubagentEffort, ToolDetail};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Subagents of a turn, by the id of the call that started each.
#[derive(Default)]
pub(crate) struct Subagents {
    /// What the turn's own agent runs on, which its subagents inherit.
    turn: Option<Run>,
    started: HashMap<String, Subagent>,
    launches: HashMap<String, Launch>,
    /// Where Claude Code keeps its sessions; `None` for its own folder.
    projects: Option<PathBuf>,
}

/// A call that started a subagent or a workflow.
struct Launch {
    /// The subagent whose call it is; `None` for the turn's own agent.
    owner: Option<String>,
    name: String,
    /// As the call's `ToolCallStarted` reported it.
    input: String,
}

#[derive(Clone)]
struct Run {
    model: String,
    effort: Option<Effort>,
}

struct Subagent {
    /// The subagent whose call started it; `None` for the turn's own agent.
    owner: Option<String>,
    run: Run,
    /// What it was asked for or inherited, which the model it turns out to run on may not
    /// support.
    asked_effort: Option<Effort>,
    /// One of its own messages told the model it runs on.
    seen: bool,
    /// Claude Code's id for it, which `SendMessage` resumes it by.
    agent_id: Option<String>,
}

impl Run {
    fn resolve(model: Option<&str>, effort: Option<Effort>) -> Self {
        let (model, effort) = manifest().run(model, effort);
        Self { model, effort }
    }
}

impl Subagent {
    fn detail(&self) -> ToolDetail {
        ToolDetail::Subagent {
            model: self.run.model.clone(),
            effort: self.run.effort,
            resume_id: self.agent_id.clone(),
        }
    }

    /// `output` about its call, in its owner's transcript.
    fn within_owner(&self, output: ProviderOutput) -> ProviderOutput {
        match &self.owner {
            Some(owner) => ProviderOutput::Subagent {
                tool_use_id: owner.clone(),
                output: Box::new(output),
            },
            None => output,
        }
    }
}

impl Subagents {
    /// For a turn on `model`, `None` for the default one, at `effort`.
    pub(crate) fn for_turn(model: Option<&str>, effort: Option<Effort>) -> Self {
        Self {
            turn: Some(Run::resolve(model, effort)),
            ..Self::default()
        }
    }

    /// The turn's own agent switched to `model` at `effort`; the subagents it started stay.
    pub(crate) fn retarget(&mut self, model: Option<&str>, effort: Option<Effort>) {
        self.turn = Some(Run::resolve(model, effort));
    }

    /// Subagents of a turn whose sessions Claude Code keeps under `projects`.
    #[cfg(test)]
    pub(crate) fn kept_in(projects: &Path) -> Self {
        Self {
            projects: Some(projects.to_owned()),
            ..Self::default()
        }
    }

    /// The call `id`, made by the turn's agent or by the subagent `owner`, starts a subagent or
    /// a workflow.
    pub(crate) fn launched(&mut self, owner: Option<&str>, id: &str, name: &str, input: String) {
        self.launches.insert(
            id.to_owned(),
            Launch {
                owner: owner.map(str::to_owned),
                name: name.to_owned(),
                input,
            },
        );
    }

    /// The call that first started subagent `task_id` of session `session_id`, which an
    /// earlier process ran; known from then on, to show it running again when the session's
    /// transcript still holds it, with what it runs on and its id.
    pub(crate) fn launched_earlier(&mut self, session_id: &str, task_id: &str) -> Option<String> {
        let projects = match &self.projects {
            Some(projects) => projects.clone(),
            None => install::config_dir()?.join("projects"),
        };
        let (launch, call) = launch_on_disk(&projects, session_id, task_id)?;
        if let Some((name, input)) = call {
            if matches!(name.as_str(), "Agent" | "Task") && !self.started.contains_key(&launch) {
                self.started(None, &launch, &input);
            }
            let input = truncate_json(&input, super::TOOL_INPUT_MAX_CHARS);
            self.launched(None, &launch, &name, input);
        }
        if let Some(subagent) = self.started.get_mut(&launch) {
            subagent.agent_id.get_or_insert_with(|| task_id.to_owned());
        }
        Some(launch)
    }

    /// Whether call `id` started a subagent or a workflow; `Some(owner)` when it did, the
    /// owner being the subagent that made the call.
    pub(crate) fn owner(&self, id: &str) -> Option<Option<&str>> {
        self.launches.get(id).map(|launch| launch.owner.as_deref())
    }

    /// Call `id` runs again, as its subagent was resumed: the call, started anew, with what it
    /// runs on when known.
    pub(crate) fn reopened(&self, id: &str) -> Vec<ProviderOutput> {
        let Some(launch) = self.launches.get(id) else {
            return Vec::new();
        };
        let started = ProviderOutput::ToolCallStarted {
            item_id: id.to_owned(),
            name: launch.name.clone(),
            input: launch.input.clone(),
        };
        let detail = self
            .detail(id)
            .map(|detail| ProviderOutput::ToolCallDetail {
                item_id: id.to_owned(),
                detail,
            });
        std::iter::once(started)
            .chain(detail)
            .map(|output| match &launch.owner {
                Some(owner) => ProviderOutput::Subagent {
                    tool_use_id: owner.clone(),
                    output: Box::new(output),
                },
                None => output,
            })
            .collect()
    }

    /// The `Agent` call `id`, made by the turn's agent or by the subagent `owner`, started a
    /// subagent: what it runs on, as its input asks or as it inherits.
    pub(crate) fn started(
        &mut self,
        owner: Option<&str>,
        id: &str,
        input: &Value,
    ) -> Option<ToolDetail> {
        let inherited = match owner {
            Some(owner) => self.started.get(owner).map(|subagent| subagent.run.clone()),
            None => self.turn.clone(),
        }?;
        let fork = input["subagent_type"] == "fork";
        let model = input["model"]
            .as_str()
            .filter(|model| !fork && *model != "inherit")
            .unwrap_or(&inherited.model);
        let asked_effort = input["effort"]
            .as_str()
            .filter(|_| !fork)
            .and_then(parse_effort)
            .or(inherited.effort);
        let subagent = Subagent {
            owner: owner.map(str::to_owned),
            run: Run::resolve(Some(model), asked_effort),
            asked_effort,
            seen: false,
            agent_id: None,
        };
        let detail = subagent.detail();
        self.started.insert(id.to_owned(), subagent);
        Some(detail)
    }

    /// The subagent of call `id` answered on `model`, which corrects what it was thought to
    /// run on. Says so on the call, in its owner's transcript, when that changed.
    pub(crate) fn model_seen(&mut self, id: &str, model: Option<&str>) -> Option<ProviderOutput> {
        let model = model.filter(|model| !model.is_empty() && *model != "<synthetic>")?;
        let subagent = self.started.get_mut(id).filter(|subagent| !subagent.seen)?;
        subagent.seen = true;
        let run = Run::resolve(Some(model), subagent.asked_effort);
        if run.model == subagent.run.model && run.effort == subagent.run.effort {
            return None;
        }
        subagent.run = run;
        Some(subagent.within_owner(ProviderOutput::ToolCallDetail {
            item_id: id.to_owned(),
            detail: subagent.detail(),
        }))
    }

    /// Claude Code named the subagent of call `id` `agent_id`. Says so on the call, in its
    /// owner's transcript, the first time.
    pub(crate) fn identified(&mut self, id: &str, agent_id: &str) -> Option<ProviderOutput> {
        let subagent = self.started.get_mut(id).filter(|subagent| {
            !agent_id.is_empty() && subagent.agent_id.as_deref() != Some(agent_id)
        })?;
        subagent.agent_id = Some(agent_id.to_owned());
        Some(subagent.within_owner(ProviderOutput::ToolCallDetail {
            item_id: id.to_owned(),
            detail: subagent.detail(),
        }))
    }

    /// What the subagent of call `id` runs on and its id, when it started one.
    pub(crate) fn detail(&self, id: &str) -> Option<ToolDetail> {
        self.started.get(id).map(Subagent::detail)
    }
}

/// The call that first started a subagent of an earlier process, as Claude Code keeps it
/// under `projects`: the call's id, and its name and input when the session's own
/// transcript holds it.
pub(crate) fn launch_on_disk(
    projects: &Path,
    session_id: &str,
    task_id: &str,
) -> Option<(String, Option<(String, Value)>)> {
    let plain = |id: &str| {
        !id.is_empty()
            && id
                .chars()
                .all(|char| char.is_ascii_alphanumeric() || char == '-' || char == '_')
    };
    if !plain(session_id) || !plain(task_id) {
        return None;
    }
    let meta = format!("agent-{task_id}.meta.json");
    let (folder, launch) = std::fs::read_dir(projects)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .find_map(|folder| {
            let path = folder.join(session_id).join("subagents").join(&meta);
            let meta: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
            let launch = meta["toolUseId"].as_str()?.to_owned();
            Some((folder, launch))
        })?;
    let transcript = std::fs::read_to_string(folder.join(format!("{session_id}.jsonl"))).ok();
    let call = transcript.and_then(|transcript| {
        transcript
            .lines()
            .filter(|line| line.contains(&launch))
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .find_map(|record| {
                record["message"]["content"]
                    .as_array()?
                    .iter()
                    .find(|block| block["type"] == "tool_use" && block["id"] == launch.as_str())
                    .and_then(|block| {
                        Some((block["name"].as_str()?.to_owned(), block["input"].clone()))
                    })
            })
    });
    Some((launch, call))
}

/// What tells the agent how to choose its subagents' model and effort; `None` when they
/// run at the thread's own. `delegates` when it has hoshi's `delegate_task`.
pub(crate) fn instructions(
    policy: SubagentEffort,
    model: Option<&str>,
    effort: Option<Effort>,
    delegates: bool,
) -> Option<String> {
    if policy == SubagentEffort::Thread {
        return None;
    }
    let (slug, effort) = manifest().run(model, effort);
    let name = manifest().name(&slug).unwrap_or(&slug);
    let tools = if delegates {
        "each Agent call and delegate_task"
    } else {
        "each Agent call"
    };
    let yours = match effort {
        Some(effort) => format!("{name} ({slug}) at {} effort", effort.as_str()),
        None => format!("{name} ({slug})"),
    };
    Some(format!(
        "Subagents and delegated tasks: you run on {yours}, their ceiling. These instructions ask you to set `model` and `effort` on {tools} by the work:\n\
         - Searching, reading or summarizing: model \"haiku\", effort \"low\".\n\
         - A focused, well-defined change: model \"sonnet\", effort \"medium\".\n\
         - Hard design, debugging, review or anything ambiguous: omit both to inherit yours.\n\
         Never above yours; the user's choice wins. Forks inherit yours."
    ))
}

#[cfg(test)]
mod tests;
