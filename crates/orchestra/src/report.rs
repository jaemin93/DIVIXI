//! The worker's report: what crosses the membrane when a worker's turn ends.
//!
//! A worker ends each turn with a ```` ```divixi-report ```` block holding
//! JSON. [`parse`] finds and checks it; the conductor then gets
//! [`Report::for_conductor`], a short structured account, instead of the
//! worker's whole reply. A reply without a usable block gets one reminder
//! (see [`reminder`]); after that the app falls back to
//! [`Report::unstructured`].

use serde::{Deserialize, Serialize};

/// The fence that opens a report block.
pub const FENCE: &str = "```divixi-report";

/// Where the work stands, in the worker's words.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Standing {
    /// The task is finished.
    Done,
    /// Part of it is finished.
    Partial,
    /// The worker cannot go on without an answer (see `questions`).
    Blocked,
    /// The work did not succeed.
    Failed,
}

impl Standing {
    pub fn as_str(self) -> &'static str {
        match self {
            Standing::Done => "done",
            Standing::Partial => "partial",
            Standing::Blocked => "blocked",
            Standing::Failed => "failed",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "done" | "complete" | "completed" => Standing::Done,
            "partial" | "partially_done" => Standing::Partial,
            "blocked" => Standing::Blocked,
            "failed" | "error" => Standing::Failed,
            _ => return None,
        })
    }
}

/// A file the worker changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    pub path: String,
    #[serde(default)]
    pub what: String,
}

/// How a check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    NotRun,
}

impl Outcome {
    fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().replace([' ', '-'], "_").as_str() {
            "pass" | "passed" | "ok" | "success" => Outcome::Pass,
            "fail" | "failed" | "error" => Outcome::Fail,
            "not_run" | "skipped" | "skip" | "not_run_yet" => Outcome::NotRun,
            _ => return None,
        })
    }
}

/// Something the worker ran to verify its work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub what: String,
    pub result: Outcome,
    #[serde(default)]
    pub detail: String,
}

/// A worker turn's report, checked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Report {
    pub status: Standing,
    pub summary: String,
    #[serde(default)]
    pub changes: Vec<Change>,
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub risks: Vec<String>,
    /// What the conductor or the human must decide.
    #[serde(default)]
    pub questions: Vec<String>,
    #[serde(default)]
    pub next: Vec<String>,
    /// Whether the worker wrote it (false: the app made it from the reply).
    #[serde(default)]
    pub structured: bool,
    /// Why the app had to make it, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// Files the app saw the worker's edit tools touch, whatever it claims.
    #[serde(default)]
    pub edits_seen: Vec<String>,
    /// The run that asked again for the block, when the first reply had none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder_run: Option<String>,
}

const MAX_SUMMARY: usize = 800;
const MAX_TEXT: usize = 400;
const MAX_ITEMS: usize = 30;

fn clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(max).collect::<String>())
    }
}

/// Strings from a JSON array (strings, or objects' first string field),
/// empty ones dropped.
fn strings(v: Option<&serde_json::Value>) -> Vec<String> {
    let Some(serde_json::Value::Array(items)) = v else { return Vec::new() };
    items
        .iter()
        .filter_map(|i| match i {
            serde_json::Value::String(s) => Some(s.clone()),
            serde_json::Value::Object(o) => o.values().find_map(|v| v.as_str().map(str::to_string)),
            _ => None,
        })
        .map(|s| clip(&s, MAX_TEXT))
        .filter(|s| !s.is_empty())
        .take(MAX_ITEMS)
        .collect()
}

fn text(o: &serde_json::Map<String, serde_json::Value>, keys: &[&str]) -> String {
    keys.iter().find_map(|k| o.get(*k).and_then(|v| v.as_str())).unwrap_or_default().to_string()
}

/// The JSON inside the last report block of a reply.
fn block(reply: &str) -> Result<&str, String> {
    let at = reply.rfind(FENCE).ok_or_else(|| format!("no {FENCE} block"))?;
    let body = &reply[at + FENCE.len()..];
    let body = body.strip_prefix('\r').unwrap_or(body);
    let body = body.strip_prefix('\n').ok_or_else(|| format!("{FENCE} must be followed by a new line"))?;
    let end = body.find("```").ok_or_else(|| "the report block is not closed with ```".to_string())?;
    Ok(body[..end].trim())
}

/// The report in a worker's reply, or why there is none usable.
pub fn parse(reply: &str) -> Result<Report, String> {
    let json = block(reply)?;
    let value: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("the report block is not valid JSON: {e}"))?;
    let o = value.as_object().ok_or("the report block must be a JSON object")?;
    let status_text = text(o, &["status"]);
    let status = Standing::parse(&status_text).ok_or_else(|| format!("status must be done, partial, blocked or failed (got {status_text:?})"))?;
    let summary = clip(&text(o, &["summary"]), MAX_SUMMARY);
    if summary.is_empty() {
        return Err("summary is empty".to_string());
    }
    let changes = match o.get("changes") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|i| match i {
                serde_json::Value::String(p) => Some(Change { path: clip(p, MAX_TEXT), what: String::new() }),
                serde_json::Value::Object(c) => {
                    let path = clip(&text(c, &["path", "file"]), MAX_TEXT);
                    (!path.is_empty()).then(|| Change { path, what: clip(&text(c, &["what", "change", "description"]), MAX_TEXT) })
                }
                _ => None,
            })
            .take(MAX_ITEMS)
            .collect(),
        _ => Vec::new(),
    };
    let checks = match o.get("checks") {
        Some(serde_json::Value::Array(items)) => items
            .iter()
            .filter_map(|i| {
                let c = i.as_object()?;
                let what = clip(&text(c, &["what", "check", "command"]), MAX_TEXT);
                let result = Outcome::parse(&text(c, &["result", "status"]))?;
                (!what.is_empty()).then(|| Check { what, result, detail: clip(&text(c, &["detail", "note"]), MAX_TEXT) })
            })
            .take(MAX_ITEMS)
            .collect(),
        _ => Vec::new(),
    };
    let questions = strings(o.get("questions"));
    if status == Standing::Blocked && questions.is_empty() {
        return Err("status is blocked but questions is empty: say what you need answered".to_string());
    }
    Ok(Report {
        status,
        summary,
        changes,
        checks,
        risks: strings(o.get("risks")),
        questions,
        next: strings(o.get("next")),
        structured: true,
        problem: None,
        edits_seen: Vec::new(),
        reminder_run: None,
    })
}

impl Report {
    /// A report made by the app from a reply without a usable block.
    pub fn unstructured(finished: bool, reply: &str, problem: &str) -> Report {
        let summary = clip(reply, MAX_SUMMARY);
        Report {
            status: if finished { Standing::Partial } else { Standing::Failed },
            summary: if summary.is_empty() { "(no text output)".to_string() } else { summary },
            changes: Vec::new(),
            checks: Vec::new(),
            risks: Vec::new(),
            questions: Vec::new(),
            next: Vec::new(),
            structured: false,
            problem: Some(problem.to_string()),
            edits_seen: Vec::new(),
            reminder_run: None,
        }
    }

    /// The report as the conductor reads it: short, every section named.
    pub fn for_conductor(&self, run: &str) -> String {
        let mut out = format!("status: {}\nsummary: {}\n", self.status.as_str(), self.summary);
        if !self.structured {
            out.push_str(&format!(
                "(The worker gave no report block: {}. The summary above is the start of its reply; read_report(\"{run}\") has it all.)\n",
                self.problem.as_deref().unwrap_or("unknown")
            ));
        }
        if !self.changes.is_empty() {
            out.push_str("changes:\n");
            for c in &self.changes {
                if c.what.is_empty() {
                    out.push_str(&format!("- {}\n", c.path));
                } else {
                    out.push_str(&format!("- {} — {}\n", c.path, c.what));
                }
            }
        }
        if self.structured {
            if self.checks.is_empty() {
                out.push_str("checks: none (nothing was verified)\n");
            } else {
                out.push_str("checks:\n");
                for c in &self.checks {
                    let mark = match c.result {
                        Outcome::Pass => "pass",
                        Outcome::Fail => "FAIL",
                        Outcome::NotRun => "not run",
                    };
                    let detail = if c.detail.is_empty() { String::new() } else { format!(" — {}", c.detail) };
                    out.push_str(&format!("- [{mark}] {}{detail}\n", c.what));
                }
            }
        }
        for (name, items) in [("risks", &self.risks), ("questions", &self.questions), ("next", &self.next)] {
            if !items.is_empty() {
                out.push_str(&format!("{name}:\n"));
                for i in items {
                    out.push_str(&format!("- {i}\n"));
                }
            }
        }
        // What the app saw, to hold the claims against.
        let claimed: Vec<&str> = self.changes.iter().map(|c| c.path.as_str()).collect();
        let unclaimed: Vec<&String> = self
            .edits_seen
            .iter()
            .filter(|seen| !claimed.iter().any(|c| seen.ends_with(c) || c.ends_with(seen.as_str())))
            .collect();
        if !unclaimed.is_empty() {
            out.push_str("edits the app saw that the report does not list:\n");
            for e in unclaimed {
                out.push_str(&format!("- {e}\n"));
            }
        }
        out.trim_end().to_string()
    }
}

/// What every task to a worker ends with: how to report.
pub fn instructions() -> String {
    format!(
        r#"---
When you finish this turn, end your reply with a report for the conductor: a {FENCE} block holding JSON, exactly in this shape. Write its text in the language of the task.

{FENCE}
{{
  "status": "done",
  "summary": "One or two sentences: what you did and where it stands.",
  "changes": [{{ "path": "src/parser.rs", "what": "handles empty input" }}],
  "checks": [{{ "what": "cargo test -p parser", "result": "pass", "detail": "" }}],
  "risks": [],
  "questions": [],
  "next": []
}}
```

- status: done (the task is finished), partial (part of it), blocked (you cannot go on without an answer; put it in questions), failed.
- changes: files you changed (not files you only read).
- checks: what you ran to verify, each pass, fail or not_run. Leave it empty if you verified nothing; do not claim checks you did not run.
- risks: what might be wrong or break. questions: what the conductor or the human must decide. next: what should come after.
- The block comes last; nothing after it."#
    )
}

/// The follow-up that asks again for a missing or broken block.
pub fn reminder(problem: &str) -> String {
    format!(
        "Your reply did not end with a usable report ({problem}). Do no more work. Reply with only the {FENCE} block for what you just did, in the shape you were given."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"I fixed it.

```divixi-report
{
  "status": "done",
  "summary": "빈 입력을 처리하도록 파서를 고쳤습니다.",
  "changes": [{ "path": "src/parser.rs", "what": "empty input" }, "README.md"],
  "checks": [{ "what": "cargo test", "result": "passed" }, { "what": "clippy", "result": "skipped" }],
  "risks": ["large files untested"],
  "questions": [],
  "next": ["add a fuzz test"]
}
```
"#;

    #[test]
    fn reads_a_good_block() {
        let r = parse(GOOD).unwrap();
        assert_eq!(r.status, Standing::Done);
        assert!(r.structured);
        assert_eq!(r.changes.len(), 2);
        assert_eq!(r.changes[1].path, "README.md");
        assert_eq!(r.checks[0].result, Outcome::Pass);
        assert_eq!(r.checks[1].result, Outcome::NotRun);
        assert_eq!(r.next, vec!["add a fuzz test"]);
    }

    #[test]
    fn the_last_block_counts() {
        let two = format!("{}\n{}", GOOD.replace("\"done\"", "\"partial\""), GOOD);
        assert_eq!(parse(&two).unwrap().status, Standing::Done);
    }

    #[test]
    fn says_what_is_wrong() {
        assert!(parse("all done").unwrap_err().contains("no ```divixi-report"));
        assert!(parse("```divixi-report\n{ oops }\n```").unwrap_err().contains("not valid JSON"));
        assert!(parse("```divixi-report\n{\"status\":\"great\",\"summary\":\"x\"}\n```").unwrap_err().contains("status must be"));
        assert!(parse("```divixi-report\n{\"status\":\"done\",\"summary\":\"  \"}\n```").unwrap_err().contains("summary"));
        assert!(parse("```divixi-report\n{\"status\":\"blocked\",\"summary\":\"x\"}\n```").unwrap_err().contains("questions"));
        assert!(parse("```divixi-report\n{\"status\":\"done\",\"summary\":\"x\"}").unwrap_err().contains("not closed"));
    }

    #[test]
    fn the_conductor_reads_claims_and_what_was_seen() {
        let mut r = parse(GOOD).unwrap();
        r.edits_seen = vec!["C:/w/src/parser.rs".into(), "C:/w/src/lexer.rs".into()];
        let text = r.for_conductor("t3");
        assert!(text.starts_with("status: done\nsummary: 빈 입력"));
        assert!(text.contains("- src/parser.rs — empty input"));
        assert!(text.contains("- [pass] cargo test"));
        assert!(text.contains("- [not run] clippy"));
        assert!(text.contains("the report does not list:\n- C:/w/src/lexer.rs"), "{text}");
        assert!(!text.contains("parser.rs\n- C:/w/src/parser.rs"));
    }

    #[test]
    fn no_checks_is_said_out_loud() {
        let r = parse("```divixi-report\n{\"status\":\"done\",\"summary\":\"x\"}\n```").unwrap();
        assert!(r.for_conductor("t1").contains("checks: none"));
        let u = Report::unstructured(true, "some reply", "no block");
        assert!(!u.structured);
        assert!(u.for_conductor("t1").contains("read_report(\"t1\")"));
    }

    #[test]
    fn instructions_parse_as_their_own_example() {
        let r = parse(&instructions()).unwrap();
        assert_eq!(r.status, Standing::Done);
    }
}
