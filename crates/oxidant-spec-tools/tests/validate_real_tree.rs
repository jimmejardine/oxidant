use std::collections::BTreeMap;
use std::path::PathBuf;

use oxidant_spec_tools::{Warning, WarningKind, validate};

fn repo_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is the crate dir; the repo root is two levels up.
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
fn validate_runs_on_real_spec_tree() {
    let repo = repo_root();
    let warnings = validate(&repo);

    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for w in &warnings {
        *counts.entry(format!("{:?}", w.kind)).or_default() += 1;
    }

    eprintln!("\n--- validate({}) summary ---", repo.display());
    eprintln!("total warnings: {}", warnings.len());
    for (kind, count) in &counts {
        eprintln!("  {kind:30} {count}");
    }

    // Smoke check: the function returns. We do not assert on counts because
    // they are the drift baseline; this test will be useful for tracking it
    // by running with `cargo test -p oxidant-spec-tools -- --nocapture`.
    assert!(!warnings.iter().any(|w| {
        // A parse error against any of our own specs would mean the parser
        // doesn't understand a real-world frontmatter shape — that we DO want
        // to fail on.
        matches!(w.kind, WarningKind::ParseError)
            && w.spec_id.as_deref().is_some_and(|id| !id.starts_with("scratch/"))
    }), "parse error on a non-scratch spec — parser cannot handle real input");

    sample_diagnostics(&warnings);
}

fn sample_diagnostics(warnings: &[Warning]) {
    eprintln!("\n--- first 5 of each kind ---");
    let mut by_kind: BTreeMap<String, Vec<&Warning>> = BTreeMap::new();
    for w in warnings {
        by_kind.entry(format!("{:?}", w.kind)).or_default().push(w);
    }
    for (kind, ws) in &by_kind {
        eprintln!("[{kind}]");
        for w in ws.iter().take(5) {
            let id = w.spec_id.as_deref().unwrap_or("<tree>");
            eprintln!("  {id}: {}", w.message);
        }
    }
}
