//! `judge notes` judges new notes; `judge digest` writes the digest.
//! Both read and write `data/` in the current checkout; the workflow commits.
//!
//! Environment: GEMINI_API_KEY (required), JUDGE_MODEL, DIGEST_MODEL,
//! GEMINI_BASE_URL (tests only).

use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use aarambh_core::Timestamp;
use judge::data::Repo;
use judge::digest::{DigestError, Outcome, write_digest};
use judge::gemini::Gemini;
use judge::llm::LlmError;
use judge::notes::judge_notes;

const DEFAULT_JUDGE_MODEL: &str = "gemini-3.5-flash-lite";
const DEFAULT_DIGEST_MODEL: &str = "gemini-3.8-flash";
const USAGE: &str = "usage: judge <notes|digest> [--data DIR] [--max N]";

fn env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

fn now() -> Timestamp {
    Timestamp(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
    )
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut data = "data".to_owned();
    let mut max = None;
    let mut i = 1;
    while i < args.len() {
        match (args[i].as_str(), args.get(i + 1)) {
            ("--data", Some(v)) => data = v.clone(),
            ("--max", Some(v)) => max = v.parse().ok(),
            _ => {
                eprintln!("{USAGE}");
                return ExitCode::from(2);
            }
        }
        i += 2;
    }
    let repo = Repo::new(data);

    let gemini = match Gemini::new(
        env("GEMINI_API_KEY").unwrap_or_default(),
        env("GEMINI_BASE_URL"),
    ) {
        Ok(g) => g,
        Err(LlmError::MissingKey) => {
            // Not an error: the repo may not have the secret yet.
            println!("::warning::GEMINI_API_KEY is not set, so nothing was judged or summarised.");
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };

    match args.first().map(String::as_str) {
        Some("notes") => {
            let model = env("JUDGE_MODEL").unwrap_or_else(|| DEFAULT_JUDGE_MODEL.into());
            match judge_notes(&gemini, &repo, &model, now(), max).await {
                Ok(r) => {
                    println!(
                        "judged {} notes with {model}, {} without a model call; {} left for later",
                        r.judged, r.without_call, r.remaining
                    );
                    for (id, e) in &r.failed {
                        println!("::warning::note {id} was not judged: {e}");
                    }
                    if let Some(why) = r.stopped {
                        println!("::warning::stopped early: {why}");
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("digest") => {
            let mut model = env("DIGEST_MODEL").unwrap_or_else(|| DEFAULT_DIGEST_MODEL.into());
            let mut result = write_digest(&gemini, &repo, &model, now()).await;
            // A busy or unreachable model: try the lighter judge model once.
            let fallback = env("JUDGE_MODEL").unwrap_or_else(|| DEFAULT_JUDGE_MODEL.into());
            if let Err(DigestError::Llm(e @ (LlmError::Unavailable(_) | LlmError::RateLimited))) =
                &result
                && fallback != model
            {
                println!("::warning::{model} failed ({e}); trying {fallback}");
                model = fallback;
                result = write_digest(&gemini, &repo, &model, now()).await;
            }
            match result {
                Ok(Outcome::NothingNew) => {
                    println!("nothing new since the last digest; skipped");
                    ExitCode::SUCCESS
                }
                Ok(Outcome::Written { path, new_events }) => {
                    println!(
                        "wrote {} covering {new_events} new events with {model}",
                        path.display()
                    );
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
