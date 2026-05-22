use std::path::PathBuf;

use oxidant_spec_tools::{Drift, diff_all, diff_spec};

fn repo_root() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.parent().unwrap().parent().unwrap().to_path_buf()
}

#[test]
fn contracts_have_no_trait_drift_against_their_code() {
    let repo = repo_root();
    let drifts = diff_all(&repo);

    let trait_drifts: Vec<&Drift> = drifts
        .iter()
        .filter(|d| !matches!(d, Drift::MissingCodePath { .. }))
        .collect();

    if !trait_drifts.is_empty() {
        eprintln!("--- trait drifts found ---");
        for d in &trait_drifts {
            eprintln!("  {d:#?}");
        }
    }
    assert!(
        trait_drifts.is_empty(),
        "expected zero trait drift on landed contracts, found {}",
        trait_drifts.len()
    );
}

#[test]
fn diff_spec_scoped_to_a_known_contract_runs() {
    let repo = repo_root();
    // contracts/tool and contracts/provider have realised traits.
    let _drifts = diff_spec(&repo, "contracts/tool");
    let _drifts = diff_spec(&repo, "contracts/provider");
}

#[test]
fn diff_spec_unknown_id_returns_empty() {
    let repo = repo_root();
    let drifts = diff_spec(&repo, "contracts/nope-not-a-real-contract");
    assert!(drifts.is_empty());
}

#[test]
fn diff_all_baseline_summary() {
    // Print the current drift baseline for visibility (run with --nocapture).
    let repo = repo_root();
    let drifts = diff_all(&repo);
    let mut method_added = 0;
    let mut method_removed = 0;
    let mut sig_changed = 0;
    let mut missing = 0;
    for d in &drifts {
        match d {
            Drift::MethodAdded { .. } => method_added += 1,
            Drift::MethodRemoved { .. } => method_removed += 1,
            Drift::MethodSignatureChanged { .. } => sig_changed += 1,
            Drift::MissingCodePath { .. } => missing += 1,
        }
    }
    eprintln!("\n--- diff_all({}) summary ---", repo.display());
    eprintln!("  MethodAdded:            {method_added}");
    eprintln!("  MethodRemoved:          {method_removed}");
    eprintln!("  MethodSignatureChanged: {sig_changed}");
    eprintln!("  MissingCodePath:        {missing}");
    eprintln!("  TOTAL:                  {}", drifts.len());
}
