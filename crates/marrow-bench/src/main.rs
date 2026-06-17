use std::fs;
use std::path::Path;

use marrow_bench::{run_consoleval, run_tokeneval};
use serde_json::json;

fn main() {
    let c = run_consoleval();
    let t = run_tokeneval();

    println!("ConsolEval ({} cases)", c.cases);
    println!("  clustering precision : {:.1}%", c.precision * 100.0);
    println!("  clustering recall    : {:.1}%", c.recall * 100.0);
    println!("  false merges         : {}", c.false_merges);
    println!(
        "  survivor correct     : {}/{}",
        c.survivor_correct, c.survivor_total
    );
    println!();
    println!("TokenEval ({} memories, budget {})", t.memories, t.budget);
    println!("  full result tokens   : {}", t.full_tokens);
    println!("  budgeted tokens      : {}", t.budget_tokens);
    println!("  reduction            : {:.1}%", t.reduction_pct);

    let summary = json!({
        "consoleval": {
            "cases": c.cases,
            "precision": c.precision,
            "recall": c.recall,
            "false_merges": c.false_merges,
            "survivor_correct": c.survivor_correct,
            "survivor_total": c.survivor_total,
        },
        "tokeneval": {
            "memories": t.memories,
            "budget": t.budget,
            "full_tokens": t.full_tokens,
            "budget_tokens": t.budget_tokens,
            "reduction_pct": t.reduction_pct,
        },
    });
    let out = Path::new("bench/results/summary.json");
    if let Some(parent) = out.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if fs::write(out, format!("{summary:#}\n")).is_ok() {
        println!("\nwrote {}", out.display());
    }
}
