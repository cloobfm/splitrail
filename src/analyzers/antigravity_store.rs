//! Antigravity CLI conversation store: `~/.gemini/antigravity-cli/conversations/<id>.db`, one
//! SQLite database per conversation, written by `agy` for interactive and print-mode runs alike.
//! Its rows are protobuf with no published schema; the field numbers below were read off real
//! conversations and checked against the exact usage `agy -p --output-format json` reports
//! (input, output, cache read and thinking matched to the token over a 128-call run).
//!
//! * `steps` (ordered by `idx`), by `step_type`:
//!   * 14, user input: text at 19.2 (19.3.1 as fallback), workspace at 19.12.1.42.11.1.
//!   * 15, model response: text at 20.3, one 20.7 per tool call, and that call's usage at 5.9:
//!     2 input, 3 output (thinking included), 5 cache read, 9 thinking, 10 visible output,
//!     7 the generation id (`bot-…`).
//!   * Every step: created at 5.1 (1 seconds, 2 nanos).
//! * `gen_metadata`: one row per model call; 1.19 the model slug (`gemini-3.8-flash-high`,
//!   `claude-sonnet-5-5-medium`), 1.4.7 the generation id joining it to its step.
//!
//! The database is in WAL mode and `agy` may hold it open, so it is copied (with its WAL) to a
//! temporary directory and read there: opening it in place would create files beside it.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use std::path::Path;

/// One decoded field: its number and either a varint or the raw bytes of a length-delimited
/// value (a string, bytes or a nested message; the reader decides which).
#[derive(Debug, Clone, Copy)]
enum Wire<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
    Fixed,
}

fn varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *buf.get(*pos)?;
        *pos += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte < 0x80 {
            return Some(value);
        }
    }
    None
}

/// The top-level fields of a message, or `None` if `buf` is not valid protobuf.
fn fields(buf: &[u8]) -> Option<Vec<(u64, Wire<'_>)>> {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < buf.len() {
        let key = varint(buf, &mut pos)?;
        let value = match key & 7 {
            0 => Wire::Varint(varint(buf, &mut pos)?),
            1 => {
                pos = pos.checked_add(8).filter(|&p| p <= buf.len())?;
                Wire::Fixed
            }
            5 => {
                pos = pos.checked_add(4).filter(|&p| p <= buf.len())?;
                Wire::Fixed
            }
            2 => {
                let len = usize::try_from(varint(buf, &mut pos)?).ok()?;
                let end = pos.checked_add(len).filter(|&e| e <= buf.len())?;
                let bytes = &buf[pos..end];
                pos = end;
                Wire::Bytes(bytes)
            }
            _ => return None,
        };
        out.push((key >> 3, value));
    }
    Some(out)
}

/// Bytes of the first field along `path` (each step a nested message).
fn bytes_at<'a>(buf: &'a [u8], path: &[u64]) -> Option<&'a [u8]> {
    let (&first, rest) = path.split_first()?;
    let found = fields(buf)?.into_iter().find_map(|(n, v)| match v {
        Wire::Bytes(b) if n == first => Some(b),
        _ => None,
    })?;
    if rest.is_empty() { Some(found) } else { bytes_at(found, rest) }
}

fn varint_at(buf: &[u8], path: &[u64]) -> Option<u64> {
    let (&last, parents) = path.split_last()?;
    let parent = if parents.is_empty() { buf } else { bytes_at(buf, parents)? };
    fields(parent)?.into_iter().find_map(|(n, v)| match v {
        Wire::Varint(x) if n == last => Some(x),
        _ => None,
    })
}

fn string_at(buf: &[u8], path: &[u64]) -> Option<String> {
    let text = std::str::from_utf8(bytes_at(buf, path)?).ok()?;
    (!text.is_empty()).then(|| text.to_string())
}

fn count_at(buf: &[u8], field: u64) -> usize {
    fields(buf).map_or(0, |f| f.iter().filter(|(n, _)| *n == field).count())
}

fn timestamp_at(buf: &[u8], path: &[u64]) -> Option<DateTime<Utc>> {
    let ts = bytes_at(buf, path)?;
    let secs = varint_at(ts, &[1])?;
    let nanos = varint_at(ts, &[2]).unwrap_or(0);
    DateTime::from_timestamp(i64::try_from(secs).ok()?, u32::try_from(nanos).ok()?)
}

/// One model call's usage.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Usage {
    pub input: u64,
    /// Thinking included, as billed.
    pub output: u64,
    pub cache_read: u64,
    pub thinking: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    User {
        at: DateTime<Utc>,
        text: Option<String>,
        cwd: Option<String>,
    },
    Response {
        at: DateTime<Utc>,
        text: Option<String>,
        tool_calls: u32,
        model: Option<String>,
        usage: Option<Usage>,
    },
}

const USER_INPUT: i64 = 14;
const PLANNER_RESPONSE: i64 = 15;

/// Decode one conversation's user and response steps, in order. `models` maps generation ids
/// to model slugs (from `gen_metadata`).
pub fn decode_steps(
    rows: &[(i64, Vec<u8>)],
    models: &HashMap<String, String>,
) -> Vec<Step> {
    let mut steps = Vec::new();
    for (step_type, payload) in rows {
        let Some(at) = timestamp_at(payload, &[5, 1]) else {
            continue;
        };
        match *step_type {
            USER_INPUT => steps.push(Step::User {
                at,
                text: string_at(payload, &[19, 2]).or_else(|| string_at(payload, &[19, 3, 1])),
                cwd: string_at(payload, &[19, 12, 1, 42, 11, 1]),
            }),
            PLANNER_RESPONSE => {
                let usage_msg = bytes_at(payload, &[5, 9]);
                let usage = usage_msg.map(|u| Usage {
                    input: varint_at(u, &[2]).unwrap_or(0),
                    output: varint_at(u, &[3]).unwrap_or(0),
                    cache_read: varint_at(u, &[5]).unwrap_or(0),
                    thinking: varint_at(u, &[9]).unwrap_or(0),
                });
                let model = usage_msg
                    .and_then(|u| string_at(u, &[7]))
                    .and_then(|id| models.get(&id).cloned());
                let response = bytes_at(payload, &[20]);
                steps.push(Step::Response {
                    at,
                    text: response.and_then(|r| string_at(r, &[3])),
                    tool_calls: response.map_or(0, |r| count_at(r, 7) as u32),
                    model,
                    usage,
                });
            }
            _ => {}
        }
    }
    steps
}

/// Generation id -> model slug, from `gen_metadata` rows.
pub fn decode_models(rows: &[Vec<u8>]) -> HashMap<String, String> {
    rows.iter()
        .filter_map(|row| {
            let meta = bytes_at(row, &[1])?;
            Some((string_at(meta, &[4, 7])?, string_at(meta, &[19])?))
        })
        .collect()
}

/// Read a conversation database by way of a temporary copy.
pub fn read_conversation(path: &Path) -> Result<Vec<Step>> {
    let dir = std::env::temp_dir().join(format!("splitrail-agy-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let name = path.file_name().context("no file name")?;
    let copy = dir.join(name);
    std::fs::copy(path, &copy).context("copy")?;
    let wal = path.with_extension("db-wal");
    let copy_wal = copy.with_extension("db-wal");
    if wal.is_file() {
        std::fs::copy(&wal, &copy_wal).context("copy WAL")?;
    }
    let result = (|| {
        let conn = rusqlite::Connection::open(&copy).context("open")?;
        let mut stmt = conn.prepare("SELECT data FROM gen_metadata")?;
        let gen_rows: Vec<Vec<u8>> = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .flatten()
            .collect();
        let mut stmt = conn.prepare(
            "SELECT step_type, step_payload FROM steps WHERE step_type IN (14, 15) \
             AND step_payload IS NOT NULL ORDER BY idx",
        )?;
        let rows: Vec<(i64, Vec<u8>)> = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .flatten()
            .collect();
        Ok(decode_steps(&rows, &decode_models(&gen_rows)))
    })();
    for leftover in [copy.clone(), copy_wal, copy.with_extension("db-shm")] {
        let _ = std::fs::remove_file(leftover);
    }
    let _ = std::fs::remove_dir(&dir); // Only if empty: another read may be using it.
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag(field: u64, wire: u64) -> Vec<u8> {
        encode_varint((field << 3) | wire)
    }
    fn encode_varint(mut v: u64) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(byte);
                return out;
            }
            out.push(byte | 0x80);
        }
    }
    fn int(field: u64, v: u64) -> Vec<u8> {
        [tag(field, 0), encode_varint(v)].concat()
    }
    fn msg(field: u64, body: &[u8]) -> Vec<u8> {
        [tag(field, 2), encode_varint(body.len() as u64), body.to_vec()].concat()
    }
    fn text(field: u64, s: &str) -> Vec<u8> {
        msg(field, s.as_bytes())
    }
    fn created(secs: u64) -> Vec<u8> {
        msg(5, &msg(1, &[int(1, secs), int(2, 500_000_000)].concat()))
    }

    #[test]
    fn decodes_a_prompt_and_a_priced_response() {
        let user = [
            int(1, 14),
            created(1_791_953_917),
            msg(19, &[text(2, "fix the build"), msg(12, &msg(1, &msg(42, &msg(11, &text(1, "/Users/x/proj")))))].concat()),
        ]
        .concat();
        let usage = [
            int(1, 1318),
            int(2, 1318),
            int(3, 652),
            int(5, 13224),
            int(9, 582),
            int(10, 70),
            text(7, "bot-1"),
        ]
        .concat();
        let response = [
            int(1, 15),
            msg(5, &[msg(1, &[int(1, 1_791_953_926)].concat()), msg(9, &usage)].concat()),
            msg(20, &[text(3, "Looking at next.config.ts"), msg(7, &text(1, "call_1")), msg(7, &text(1, "call_2"))].concat()),
        ]
        .concat();
        let gen_row = msg(1, &[msg(4, &text(7, "bot-1")), text(19, "gemini-3.8-flash-high")].concat());
        let models = decode_models(&[gen_row]);
        let steps = decode_steps(&[(14, user), (15, response), (132, vec![1, 2, 3])], &models);
        assert_eq!(steps.len(), 2);
        let Step::User { text, cwd, at } = &steps[0] else { panic!() };
        assert_eq!(text.as_deref(), Some("fix the build"));
        assert_eq!(cwd.as_deref(), Some("/Users/x/proj"));
        assert_eq!(at.timestamp_subsec_millis(), 500);
        let Step::Response { text, tool_calls, model, usage, .. } = &steps[1] else { panic!() };
        assert_eq!(text.as_deref(), Some("Looking at next.config.ts"));
        assert_eq!(*tool_calls, 2);
        assert_eq!(model.as_deref(), Some("gemini-3.8-flash-high"));
        assert_eq!(usage.clone().unwrap(), Usage { input: 1318, output: 652, cache_read: 13224, thinking: 582 });
    }

    #[test]
    fn garbage_is_skipped_not_fatal() {
        assert!(fields(&[0xff, 0xff]).is_none());
        assert!(decode_steps(&[(15, vec![0x0a, 0x50])], &HashMap::new()).is_empty());
    }
}
