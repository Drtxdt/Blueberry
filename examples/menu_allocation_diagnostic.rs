//! Independent allocation attribution; never use this as a latency benchmark.
use blueberry::{allocation_diagnostics, config::Config, menu, model::Candidate};
use serde_json::json;
use std::hint::black_box;

fn main() {
    let candidates: Vec<_> = (0..8)
        .map(|index| Candidate {
            label: format!("git 中文 action {index}"),
            insert_text: format!("git action-{index}"),
            description: "Description with spaces and 中文".into(),
            detail: "Detail line one\nDetail line two".into(),
            source: "fixture".into(),
            ..Default::default()
        })
        .collect();
    let mut results = Vec::new();
    for border in ["none", "single", "rounded"] {
        let mut config = Config::default();
        config.ui.border = border.into();
        for (width, rows) in [(0, 12), (1, 1), (2, 3), (24, 0), (24, 3), (80, 12)] {
            for details in [false, true] {
                let state = menu::MenuState {
                    incomplete: true,
                    details,
                    argument_hint: Some("参数提示"),
                    diagnostic: Some("Recoverable fixture error"),
                    ..Default::default()
                };
                let frame =
                    menu::render_with_state(&candidates, 3, "git", width, &config, state, rows);
                let before = allocation_diagnostics::snapshot();
                for _ in 0..100 {
                    black_box(menu::render_with_state(
                        &candidates,
                        3,
                        "git",
                        width,
                        &config,
                        state,
                        rows,
                    ));
                }
                let delta = allocation_diagnostics::snapshot().since(before);
                results.push(json!({
                    "border":border,"width":width,"rows":rows,"details":details,
                    "iterations":100,"allocation_requests":delta.allocations,
                    "requested_bytes":delta.requested_bytes,"reallocations":delta.reallocations,
                    "frame":{"width":frame.width,"lines":frame.lines}
                }));
            }
        }
    }
    println!(
        "{}",
        json!({"diagnostic_only":true,"timing_measured":false,"results":results})
    );
}
