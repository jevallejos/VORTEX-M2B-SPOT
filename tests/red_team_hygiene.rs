use std::fs;
use std::path::{Path, PathBuf};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn collect_files_recursive(
    dir: &Path,
    out: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    if !dir.exists() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_files_recursive(&path, out)?;
        } else if path.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

fn collect_all_governed_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let mut files = Vec::new();
    for sub in ["src", "tests", "deploy", ".github", "DOCS"] {
        collect_files_recursive(&root.join(sub), &mut files)?;
    }
    for single in [
        "Cargo.toml",
        ".gitignore",
        ".gitattributes",
        ".env.example",
        "README.md",
    ] {
        let p = root.join(single);
        if p.exists() {
            files.push(p);
        }
    }
    files.sort();
    files.dedup();
    Ok(files)
}

#[test]
fn src_has_zero_panic_unwrap_expect_todo_unimplemented_or_unsafe() -> TestResult {
    let root = repo_root();
    let mut src_files = Vec::new();
    collect_files_recursive(&root.join("src"), &mut src_files)?;
    assert!(!src_files.is_empty(), "expected Rust files under src/");

    let forbidden = [
        concat!(".un", "wrap("),
        concat!(".ex", "pect("),
        concat!("pan", "ic!("),
        concat!("to", "do!("),
        concat!("unimple", "mented!("),
        concat!("un", "safe"),
    ];

    for path in src_files {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        for pat in &forbidden {
            assert!(
                !text.contains(pat),
                "forbidden pattern '{pat}' found in {}",
                path.display()
            );
        }
    }

    Ok(())
}

#[test]
fn domain_has_zero_external_crate_imports() -> TestResult {
    let root = repo_root();
    let mut domain_files = Vec::new();
    collect_files_recursive(&root.join("src").join("domain"), &mut domain_files)?;
    assert!(
        !domain_files.is_empty(),
        "expected Rust files under src/domain/"
    );

    let forbidden_crates = [
        "ureq",
        "serde",
        "serde_json",
        "hmac",
        "sha2",
        "hex",
        "tokio",
    ];

    for path in domain_files {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        for (line_no, line) in text.lines().enumerate() {
            let tokens: Vec<&str> = line
                .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
                .filter(|t| !t.is_empty())
                .collect();

            for krate in &forbidden_crates {
                assert!(
                    !tokens.contains(krate),
                    "external crate reference '{krate}' found in {}:{}",
                    path.display(),
                    line_no + 1
                );
            }
        }
    }

    Ok(())
}

#[test]
fn governed_files_are_utf8_no_bom_pure_lf_and_zero_banned_strings() -> TestResult {
    let root = repo_root();
    let files = collect_all_governed_files(&root)?;
    assert!(!files.is_empty(), "expected governed repository files");

    let banned = [
        concat!("Haw", "kes"),
        concat!("Moallemi", "-", "Yuan"),
        concat!("Wald ", "SPRT"),
        concat!("SCAR", "-", "LOCK"),
        concat!("OFI", "/", "TFI"),
        concat!("/dev/", "shm"),
        concat!("200 ", "Hz"),
        concat!("Kauf", "man"),
        concat!("Mini", "max"),
        concat!("RSI", "/", "MACD"),
    ];
    let banned_lower: Vec<String> = banned.iter().map(|s| s.to_ascii_lowercase()).collect();

    for path in files {
        let bytes = fs::read(&path)?;
        assert!(
            !(bytes.len() >= 3 && bytes[0] == 0xEF && bytes[1] == 0xBB && bytes[2] == 0xBF),
            "UTF-8 BOM detected in {}",
            path.display()
        );
        assert!(
            !bytes.contains(&0x0D),
            "CR byte (0x0D) detected in {}",
            path.display()
        );
        let text = std::str::from_utf8(&bytes)
            .map_err(|e| format!("invalid UTF-8 in {}: {e}", path.display()))?;
        let lower = text.to_ascii_lowercase();
        for pat in &banned_lower {
            assert!(
                !lower.contains(pat),
                "banned legacy term '{pat}' found in {}",
                path.display()
            );
        }
    }

    Ok(())
}

#[test]
fn gitignore_env_example_and_secret_hygiene() -> TestResult {
    let root = repo_root();

    // 1) Verify .gitignore rules
    let gitignore = fs::read_to_string(root.join(".gitignore"))?;
    let gi_lines: Vec<&str> = gitignore.lines().map(str::trim).collect();
    assert!(gi_lines.contains(&".env"), ".gitignore must exclude .env");
    assert!(
        gi_lines.contains(&"state/"),
        ".gitignore must exclude state/"
    );
    assert!(gi_lines.contains(&"logs/"), ".gitignore must exclude logs/");

    // 2) Verify .env.example contains exact empty placeholders & v1.1.0 default universe
    let env_example = fs::read_to_string(root.join(".env.example"))?;
    let env_lines: Vec<&str> = env_example.lines().map(str::trim).collect();
    for required in [
        "BYBIT_API_KEY=",
        "BYBIT_API_SECRET=",
        "TELEGRAM_BOT_TOKEN=",
        "TELEGRAM_CHAT_ID=",
        "M2B_UNIVERSE_SYMBOLS=BTCUSDT,ETHUSDT,SOLUSDT,LINKUSDT,AVAXUSDT,XAUTUSDT",
    ] {
        assert!(
            env_lines.contains(&required),
            ".env.example missing required entry '{required}'"
        );
    }

    // 3) Scan all governed files for secret-like tokens
    let secret_markers = [
        concat!("AKI", "A"),
        concat!("gh", "p_"),
        concat!("-----BEGIN ", "PRIVATE KEY-----"),
        concat!("-----BEGIN RSA ", "PRIVATE KEY-----"),
        concat!("xox", "b-"),
    ];

    let files = collect_all_governed_files(&root)?;
    for path in files {
        let text = fs::read_to_string(&path)?;
        for marker in &secret_markers {
            assert!(
                !text.contains(marker),
                "secret-like marker '{marker}' found in {}",
                path.display()
            );
        }
        if looks_like_telegram_bot_token(&text) {
            return Err(format!(
                "potential Telegram bot token literal found in {}",
                path.display()
            )
            .into());
        }
    }

    Ok(())
}

#[test]
fn v111_release_metadata_and_regression_guards() -> TestResult {
    let root = repo_root();

    let cargo_toml = fs::read_to_string(root.join("Cargo.toml"))?;
    assert!(
        cargo_toml.contains("version = \"1.1.1\""),
        "Cargo.toml must specify version = \"1.1.1\""
    );

    let timer = fs::read_to_string(root.join("deploy").join("vortex-m2b-spot.timer"))?;
    assert!(
        timer.contains("OnCalendar=*-*-* *:00:15 UTC"),
        "timer must preserve OnCalendar=*-*-* *:00:15 UTC"
    );
    assert!(
        timer.contains("RandomizedDelaySec=10"),
        "timer must preserve RandomizedDelaySec=10"
    );

    let mut src_files = Vec::new();
    collect_files_recursive(&root.join("src"), &mut src_files)?;
    let legacy_const = concat!("UNIVERSE", "_LEN");
    for path in src_files {
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let text = fs::read_to_string(&path)?;
            assert!(
                !text.contains(legacy_const),
                "legacy fixed-universe symbol '{legacy_const}' found in {}",
                path.display()
            );
        }
    }

    let types_rs = fs::read_to_string(root.join("src").join("domain").join("types.rs"))?;
    assert!(types_rs.contains("pub const STATE_SCHEMA_VERSION: u32 = 2;"));
    assert!(types_rs.contains("XAUTUSDT"));

    let strategy_rs = fs::read_to_string(root.join("src").join("domain").join("strategy.rs"))?;
    assert!(
        !strategy_rs.contains("LIQUIDATE_CURRENT_THRESH"),
        "v1.1.1 must remove LIQUIDATE_CURRENT_THRESH liquidation slit"
    );
    assert!(
        strategy_rs.contains("pub const LIQUIDATE_TARGET_THRESH: f64 = 0.015;"),
        "strategy.rs must preserve pub const LIQUIDATE_TARGET_THRESH: f64 = 0.015;"
    );
    assert!(
        strategy_rs.contains("target_w < LIQUIDATE_TARGET_THRESH && current_w > 0.0"),
        "strategy.rs must liquidate any positive weight when target_w < LIQUIDATE_TARGET_THRESH"
    );

    Ok(())
}

fn looks_like_telegram_bot_token(text: &str) -> bool {
    let bytes = text.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        if b != b':' || idx < 8 || idx + 30 > bytes.len() {
            continue;
        }
        let prefix_digits = bytes[..idx]
            .iter()
            .rev()
            .take_while(|c| c.is_ascii_digit())
            .count();
        let boundary_ok =
            idx == prefix_digits || !bytes[idx - prefix_digits - 1].is_ascii_alphanumeric();
        if prefix_digits < 8 || !boundary_ok {
            continue;
        }
        let suffix_len = bytes[idx + 1..]
            .iter()
            .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_' || **c == b'-')
            .count();
        if suffix_len >= 30 {
            return true;
        }
    }
    false
}
