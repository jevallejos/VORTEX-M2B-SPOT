use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::process::ExitCode;

use vortex_m2b_spot::application::cycle::{CycleConfig, CycleOutcome, RunHourlyCycleUseCase};
use vortex_m2b_spot::domain::accounting::SpotPortfolioLedger;
use vortex_m2b_spot::domain::error::AppError;
use vortex_m2b_spot::domain::ports::{ClockPort, MarketDataPort, StateStorePort};
use vortex_m2b_spot::domain::types::{ExecutionMode, Symbol, RATE_SCALE_1E10, UNIVERSE_LEN};
use vortex_m2b_spot::infrastructure::bybit_spot_v5::{
    BybitSpotV5Client, PaperNoopExecutionPort, SystemClock,
};
use vortex_m2b_spot::infrastructure::config::AppConfig;
use vortex_m2b_spot::infrastructure::state_store::FsStateStore;
use vortex_m2b_spot::infrastructure::telegram::TelegramNotifier;

fn print_usage() {
    println!("vortex-m2b-spot {}", env!("CARGO_PKG_VERSION"));
    println!("Usage:");
    println!("  vortex-m2b-spot run [--paper | --live-spot]");
    println!("  vortex-m2b-spot status");
    println!("  vortex-m2b-spot health");
    println!("  vortex-m2b-spot --help | --version");
}

fn parse_run_mode(args: &[String]) -> Result<ExecutionMode, String> {
    match args {
        [] => Ok(ExecutionMode::Paper),
        [flag] if flag == "--paper" => Ok(ExecutionMode::Paper),
        [flag] if flag == "--live-spot" => Ok(ExecutionMode::LiveSpot),
        _ => Err(format!("invalid flags for `run`: {}", args.join(" "))),
    }
}

fn cmd_run(args: &[String]) -> Result<(), String> {
    let mode = parse_run_mode(args)?;
    let cfg = AppConfig::load().map_err(|e| format!("config load failed: {e}"))?;
    let store = FsStateStore::new(&cfg.state_dir, &cfg.logs_dir)
        .map_err(|e| format!("state store init failed: {e}"))?;
    let clock = SystemClock;
    let market = BybitSpotV5Client::from_config(&cfg);
    let notifier = TelegramNotifier::new(&cfg.telegram_bot_token, &cfg.telegram_chat_id);
    let noop_exec = PaperNoopExecutionPort;

    let exec_port: &dyn vortex_m2b_spot::domain::ports::SpotExecutionPort = match mode {
        ExecutionMode::Paper => &noop_exec,
        ExecutionMode::LiveSpot => &market,
    };

    let use_case = RunHourlyCycleUseCase::new(&clock, &market, exec_port, &store, &notifier);
    let cycle_cfg = CycleConfig {
        mode,
        paper_initial_cash_1e8: cfg.paper_initial_cash_1e8,
        live_execution_env_flag: cfg.live_execution_env_flag,
        live_ack_valid: cfg.live_ack_valid,
        max_clock_skew_ms: cfg.max_clock_skew_ms,
        recv_window_ms: cfg.recv_window_ms,
    };

    match use_case.run(&cycle_cfg) {
        Ok(CycleOutcome::AlreadyProcessed { closed_hour_ts_ms }) => {
            println!(
                "CYCLE_ALREADY_PROCESSED mode={} closed_hour_ts_ms={}",
                mode, closed_hour_ts_ms
            );
            Ok(())
        }
        Ok(CycleOutcome::Executed {
            closed_hour_ts_ms,
            orders_executed,
            equity_1e8,
            drift_1e8,
            shadow_4h_updated,
            halted,
            drain_mode,
        }) => {
            println!(
                "CYCLE_EXECUTED mode={} closed_hour_ts_ms={} orders={} equity_usdt={} drift_1e8={} shadow_4h_updated={} halted={} drain_mode={}",
                mode,
                closed_hour_ts_ms,
                orders_executed,
                equity_1e8.format_fixed(2),
                drift_1e8,
                shadow_4h_updated,
                halted,
                drain_mode
            );
            Ok(())
        }
        Err(AppError::LiveSafetyLockDenied(msg)) => Err(format!("LIVE_SAFETY_LOCK_DENIED: {msg}")),
        Err(AppError::HaltLockActive(msg)) => Err(format!("HALT_LOCK_ACTIVE: {msg}")),
        Err(e) => Err(format!("cycle failed: {e}")),
    }
}

fn cmd_status() -> Result<(), String> {
    let cfg = AppConfig::load().map_err(|e| format!("config load failed: {e}"))?;
    let store = FsStateStore::new(&cfg.state_dir, &cfg.logs_dir)
        .map_err(|e| format!("state store init failed: {e}"))?;

    let drain_active = store
        .has_drain_lock()
        .map_err(|e| format!("failed to check DRAIN_MODE.lock: {e}"))?;
    let halt_active = store
        .has_halt_lock()
        .map_err(|e| format!("failed to check HALT_LOCK: {e}"))?;

    println!("=== VORTEX-M2B-SPOT STATUS ===");
    println!("State dir        : {}", store.state_dir().display());
    println!("Logs dir         : {}", store.logs_dir().display());
    println!("DRAIN_MODE.lock  : {}", drain_active);
    println!("HALT_LOCK        : {}", halt_active);

    match store
        .load_primary_state()
        .map_err(|e| format!("failed to load primary state: {e}"))?
    {
        None => {
            println!("Primary B2-01    : not initialized yet (no primary_state.json)");
        }
        Some(st) => {
            let ledger = SpotPortfolioLedger::from_snapshot(st.portfolio.clone())
                .map_err(|e| format!("invalid primary portfolio snapshot: {e}"))?;
            let equity = ledger
                .equity_1e8(&st.last_prices_1e8)
                .map_err(|e| format!("equity calculation error: {e}"))?;
            let dd_1e10 = ledger
                .current_drawdown_1e10()
                .map_err(|e| format!("drawdown calculation error: {e}"))?;
            let dd_pct = (dd_1e10 as f64) * 100.0 / (RATE_SCALE_1E10 as f64);
            let drift = ledger
                .drift_mtm_1e8(&st.last_prices_1e8)
                .map_err(|e| format!("drift calculation error: {e}"))?;

            println!();
            println!("--- Primary B2-01 (1h) ---");
            println!("Mode             : {}", st.mode);
            println!("Last Hour TS (ms): {}", st.last_processed_hour_ts_ms);
            println!("Cash (USDT)      : {}", ledger.cash_1e8().format_fixed(4));
            println!("Equity (USDT)    : {}", equity.format_fixed(4));
            println!(
                "Deposits (USDT)  : {}",
                ledger.total_deposits_1e8().format_fixed(4)
            );
            println!("TWR Drawdown     : {:.4}%", dd_pct);
            println!("Drift (1e8)      : {}", drift);
            println!("Halted           : {}", ledger.is_halted());
            println!();
            println!(
                "{:<10} {:>14} {:>12} {:>10} {:>10} {:>10} {:>10}",
                "SYMBOL", "QTY", "PRICE", "CUR_W", "TGT_W", "SIGMA_ANN", "TSMOM"
            );
            for i in 0..UNIVERSE_LEN {
                let sym = Symbol::ALL[i];
                let pos = ledger.position(sym);
                println!(
                    "{:<10} {:>14} {:>12} {:>9.2}% {:>9.2}% {:>9.2}% {:>10.4}",
                    sym.as_str(),
                    pos.qty_1e8.format_fixed(6),
                    st.last_prices_1e8[i].format_fixed(4),
                    st.last_current_weights[i] * 100.0,
                    st.last_target_weights[i] * 100.0,
                    st.last_sigma_ann[i] * 100.0,
                    st.last_tsmom[i]
                );
            }
        }
    }

    match store
        .load_shadow_4h_state()
        .map_err(|e| format!("failed to load shadow 4h state: {e}"))?
    {
        None => {
            println!();
            println!("Shadow A0_4H     : not initialized yet (no m2b_shadow_4h_state.json)");
        }
        Some(sh) => {
            let ledger = SpotPortfolioLedger::from_snapshot(sh.portfolio.clone())
                .map_err(|e| format!("invalid shadow portfolio snapshot: {e}"))?;
            let equity = ledger
                .equity_1e8(&sh.last_prices_1e8)
                .map_err(|e| format!("shadow equity calculation error: {e}"))?;
            let dd_1e10 = ledger
                .current_drawdown_1e10()
                .map_err(|e| format!("shadow drawdown calculation error: {e}"))?;
            let dd_pct = (dd_1e10 as f64) * 100.0 / (RATE_SCALE_1E10 as f64);
            let drift = ledger
                .drift_mtm_1e8(&sh.last_prices_1e8)
                .map_err(|e| format!("shadow drift calculation error: {e}"))?;

            println!();
            println!("--- Shadow A0_4H (4h UTC Paper) ---");
            println!("Last 4h Close TS : {}", sh.last_processed_4h_close_ts_ms);
            println!("Equity (USDT)    : {}", equity.format_fixed(4));
            println!("TWR Drawdown     : {:.4}%", dd_pct);
            println!("Drift (1e8)      : {}", drift);
        }
    }

    Ok(())
}

fn check_dir_writable(dir: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(dir)
        .map_err(|e| format!("failed to create directory {}: {e}", dir.display()))?;
    let probe_path = dir.join(format!(".health_probe_{}.tmp", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
        .map_err(|e| format!("directory {} not writable: {e}", dir.display()))?;
    file.write_all(b"ok")
        .map_err(|e| format!("write probe failed in {}: {e}", dir.display()))?;
    drop(file);
    fs::remove_file(&probe_path)
        .map_err(|e| format!("cleanup probe failed in {}: {e}", dir.display()))?;
    Ok(())
}

fn cmd_health() -> Result<(), String> {
    let cfg = AppConfig::load().map_err(|e| format!("config load failed: {e}"))?;
    check_dir_writable(&cfg.state_dir)?;
    check_dir_writable(&cfg.logs_dir)?;

    let client = BybitSpotV5Client::from_config(&cfg);
    let clock = SystemClock;
    let local_ms = clock.now_ms();
    let server_ms = client
        .fetch_server_time_ms()
        .map_err(|e| format!("Bybit V5 server time failed: {e}"))?;
    let skew_ms = local_ms.saturating_sub(server_ms).abs();
    if skew_ms > cfg.max_clock_skew_ms {
        return Err(format!(
            "clock skew {}ms exceeds max_allowed {}ms",
            skew_ms, cfg.max_clock_skew_ms
        ));
    }

    let rules = client
        .fetch_instruments_info(&Symbol::ALL)
        .map_err(|e| format!("instruments-info check failed: {e}"))?;

    println!("=== VORTEX-M2B-SPOT HEALTH OK ===");
    println!("Bybit Base URL   : {}", cfg.bybit_base_url);
    println!("Server Time (ms) : {}", server_ms);
    println!(
        "Clock Skew (ms)  : {} (limit {})",
        skew_ms, cfg.max_clock_skew_ms
    );
    println!("State Dir        : {} (rw)", cfg.state_dir.display());
    println!("Logs Dir         : {} (rw)", cfg.logs_dir.display());
    for r in rules {
        if !r.is_trading {
            return Err(format!("instrument {} is not in Trading status", r.symbol));
        }
        println!(
            "Instrument {:<8} | status=Trading | base_step={} | min_qty={} | min_amt={}",
            r.symbol.as_str(),
            r.base_precision.format_by_step(r.base_precision),
            r.min_order_qty.format_by_step(r.base_precision),
            r.min_order_amt.format_fixed(2)
        );
    }

    Ok(())
}

fn run_cli() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        print_usage();
        return Err("missing command".to_string());
    };

    match cmd {
        "run" => cmd_run(&args[1..]),
        "status" => {
            if args.len() > 1 {
                return Err("`status` takes no extra arguments".to_string());
            }
            cmd_status()
        }
        "health" => {
            if args.len() > 1 {
                return Err("`health` takes no extra arguments".to_string());
            }
            cmd_health()
        }
        "-h" | "--help" | "help" => {
            print_usage();
            Ok(())
        }
        "-V" | "--version" => {
            println!("vortex-m2b-spot {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        other => {
            print_usage();
            Err(format!("unknown command: {other}"))
        }
    }
}

fn main() -> ExitCode {
    match run_cli() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("ERROR: {msg}");
            ExitCode::FAILURE
        }
    }
}
