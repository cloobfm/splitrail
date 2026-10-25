//! Local model servers (Ollama, MTPLX) seen as servers (BZL-238): what they did for the agents
//! that call them, beside what they did directly.
//!
//! Each request is counted in one column only: an agent's own record (Pi's session, with its
//! transcript and project) wins, and the server column keeps the rest. So the totals add up by
//! construction. This module only *shows* the agent requests a server ran, read from the agent
//! columns, never adding them to anything: a server's row gets "+38 tk/s via Pi" and the
//! transcript lines of those requests, tagged with the agent.

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};

use crate::types::{AgenticCodingToolStats, ConversationMessage, MessageRole};

/// Whether a column is a local model server.
pub fn is_server(stats: &AgenticCodingToolStats) -> bool {
    !stats.analyzer_name.contains('@') && (stats.analyzer_name == "MTPLX" || crate::analyzers::ollama::ENGINE_COLUMNS.contains(&stats.analyzer_name.as_str()))
}

/// The models a server has run: its own messages' and days'. Agents name a local model by the
/// same name (Pi's `qwen-fast`, OpenCode's `qwen3.8:27b-mlx`), which is how their requests are
/// recognised as the server's.
fn models_of(server: &AgenticCodingToolStats) -> HashSet<&str> {
    server
        .messages
        .iter()
        .filter_map(|m| m.model.as_deref())
        .chain(server.daily_stats.values().flat_map(|d| d.models.keys().map(String::as_str)))
        .collect()
}

/// Requests other columns on this machine made through `server`, with the column's name.
pub fn via<'a>(
    server: &AgenticCodingToolStats,
    all: &[&'a AgenticCodingToolStats],
) -> Vec<(&'a str, &'a ConversationMessage)> {
    let models = models_of(server);
    if models.is_empty() {
        return Vec::new();
    }
    all.iter()
        .filter(|tool| !is_server(tool) && !tool.analyzer_name.contains('@'))
        .flat_map(|tool| {
            // A reply names the model; its prompt is the user message just before it.
            let mut out = Vec::new();
            let mut pending_prompt: Option<&ConversationMessage> = None;
            for m in &tool.messages {
                match m.role {
                    MessageRole::User => pending_prompt = Some(m),
                    MessageRole::Assistant => {
                        if m.model.as_deref().is_some_and(|model| models.contains(model)) {
                            if let Some(prompt) = pending_prompt.take()
                                && prompt.conversation_hash == m.conversation_hash
                            {
                                out.push((tool.analyzer_name.as_str(), prompt));
                            }
                            out.push((tool.analyzer_name.as_str(), m));
                        } else {
                            pending_prompt = None;
                        }
                    }
                }
            }
            out
        })
        .collect()
}

/// Output tk/s over the last 15 minutes of the requests made through the server, as the
/// dashboard counts its own rate.
pub fn via_tks(via: &[(&str, &ConversationMessage)], now: DateTime<Utc>) -> f64 {
    let since = now - Duration::minutes(15);
    let output: u64 = via
        .iter()
        .filter(|(_, m)| m.role == MessageRole::Assistant && m.date >= since && m.date <= now)
        .map(|(_, m)| m.stats.output_tokens)
        .sum();
    output as f64 / 900.0
}

/// The agents in `via`, by name, for "via Pi Agent".
pub fn agents(via: &[(&str, &ConversationMessage)]) -> Vec<String> {
    let mut names: Vec<String> = via.iter().map(|(name, _)| name.to_string()).collect();
    names.sort();
    names.dedup();
    names
}

/// `server` with the agents' requests through it added, for showing only: the transcript lines
/// of those requests carry the agent's name. Never fed to anything that totals.
pub fn with_via(server: &AgenticCodingToolStats, via: &[(&str, &ConversationMessage)]) -> AgenticCodingToolStats {
    let mut messages = server.messages.clone();
    messages.extend(via.iter().map(|(agent, m)| {
        let mut m = (*m).clone();
        if m.role == MessageRole::Assistant
            && let Some(content) = &m.content
        {
            m.content = Some(format!("via {agent} · {content}"));
        }
        m
    }));
    messages.sort_by_key(|m| m.date);
    AgenticCodingToolStats {
        daily_stats: Default::default(),
        num_conversations: server.num_conversations,
        messages,
        analyzer_name: server.analyzer_name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Application, Stats};

    fn msg(role: MessageRole, model: Option<&str>, output: u64, secs_ago: i64, text: &str) -> ConversationMessage {
        ConversationMessage {
            application: Application::PiAgent,
            date: Utc::now() - Duration::seconds(secs_ago),
            project_hash: "shop".into(),
            conversation_hash: "c".into(),
            local_hash: None,
            global_hash: format!("{text}{secs_ago}"),
            model: model.map(str::to_string),
            stats: Stats { output_tokens: output, ..Default::default() },
            role,
            content: Some(text.into()),
        }
    }

    fn tool(name: &str, messages: Vec<ConversationMessage>) -> AgenticCodingToolStats {
        AgenticCodingToolStats { daily_stats: Default::default(), num_conversations: 1, messages, analyzer_name: name.into() }
    }

    #[test]
    fn an_agents_requests_on_a_servers_model_are_shown_on_the_server_and_counted_nowhere_new() {
        let mut direct = msg(MessageRole::Assistant, Some("qwen-fast"), 90, 120, "bench");
        direct.application = Application::Mtplx;
        let server = tool("MTPLX", vec![direct]);
        let pi = tool(
            "Pi Agent",
            vec![
                msg(MessageRole::User, None, 0, 60, "hello"),
                msg(MessageRole::Assistant, Some("qwen-fast"), 900, 50, "Hi there"),
                msg(MessageRole::User, None, 0, 40, "use the cloud"),
                msg(MessageRole::Assistant, Some("gpt-5.5"), 500, 30, "Sure"),
            ],
        );
        let all = [&server, &pi];
        let via = via(&server, &all);
        // The local reply and its prompt; not the cloud one.
        assert_eq!(via.iter().map(|(_, m)| m.content.as_deref().unwrap()).collect::<Vec<_>>(), ["hello", "Hi there"]);
        assert_eq!(agents(&via), ["Pi Agent"]);
        assert!((via_tks(&via, Utc::now()) - 1.0).abs() < 1e-9);
        let shown = with_via(&server, &via);
        assert_eq!(shown.messages.len(), 3);
        assert_eq!(shown.messages[2].content.as_deref(), Some("via Pi Agent · Hi there"));
        // Only for showing: the server's own figures are unchanged.
        assert_eq!(server.messages.len(), 1);
    }

    #[test]
    fn remote_and_server_columns_are_not_agents() {
        let server = tool("Ollama", vec![msg(MessageRole::Assistant, Some("m"), 1, 10, "x")]);
        let remote = tool("Pi@dev-a", vec![msg(MessageRole::Assistant, Some("m"), 1, 10, "y")]);
        let other_server = tool("MTPLX", vec![msg(MessageRole::Assistant, Some("m"), 1, 10, "z")]);
        assert!(via(&server, &[&server, &remote, &other_server]).is_empty());
        assert!(!is_server(&tool("Ollama@dev-a", Vec::new())));
    }
}
