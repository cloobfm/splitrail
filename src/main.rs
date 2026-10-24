use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::sync::{Arc, Mutex};

use analyzer::AnalyzerRegistry;
use analyzers::{
    AntigravityAnalyzer, ClaudeCodeAnalyzer, CursorAgentAnalyzer, ClineAnalyzer, CodexCliAnalyzer, DroidCliAnalyzer,
    CopilotCliAnalyzer, KiloCodeAnalyzer, KiroCliAnalyzer, PiAgentAnalyzer, MtplxAnalyzer, OllamaAnalyzer, OpenCodeAnalyzer, QwenCodeAnalyzer,
    WarpDevAnalyzer,
};

mod advisor;
mod analyzer;
mod analyzers;
mod config;
mod incremental;
mod stamp_cache;
mod models;
mod mqtt;
mod rollup;
mod servers;
mod notifications;
mod plan_limits;
mod records;
mod price_feed;
mod cursor_limits;
mod droid_limits;
mod warp_limits;
mod kiro_limits;
mod mcp_server;
mod self_update;
mod status_file;
mod reqwest_simd_json;
mod tui;
mod types;
mod upload;
mod utils;
mod warp;
mod watcher;

#[derive(Parser)]
#[command(name = "splitrail-dashboard")]
#[command(version)]
#[command(disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Use comma-separated number formatting
    #[arg(long)]
    number_comma: bool,

    /// Use human-readable number formatting (k, m, b, t)
    #[arg(short = 'H', long)]
    number_human: bool,

    /// Locale for number formatting (en, de, fr, es, it, ja, ko, zh)
    #[arg(long)]
    locale: Option<String>,

    /// Number of decimal places for human-readable formatting
    #[arg(long)]
    decimal_places: Option<usize>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manually upload stats to Splitrail Cloud
    Upload,
    /// Manage configuration
    #[command(about = "Manage Splitrail configuration settings")]
    Config(ConfigArgs),
    /// Manage WARP integration (setup, refresh, status)
    #[command(about = "Manage WARP integration and auth token")]
    Warp(WarpArgs),
    /// Refresh or inspect model pricing
    #[command(about = "Refresh or inspect model pricing from the live price feed")]
    Pricing(PricingArgs),
    /// Publish live stats to the MQTT broker in `[mqtt]`, without the dashboard (for launchd)
    #[command(about = "Publish live stats to MQTT / Home Assistant without the dashboard")]
    Mqtt {
        /// Instead: clear what this machine name left on the broker (a renamed or retired
        /// machine), so its columns disappear
        #[arg(long, value_name = "HOST")]
        forget: Option<String>,
    },
    /// MCP server on stdio: quotas, where to spend, usage (needs the dashboard running)
    #[command(about = "Model Context Protocol server: plan quotas, where to spend tokens, usage")]
    Mcp,
    /// Every plan window, its reset and pace; `--json` for scripts
    #[command(about = "Show every plan quota, when it resets and where to spend next")]
    Quotas {
        /// Print the answers as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Args)]
struct PricingArgs {
    #[command(subcommand)]
    command: PricingSubcommands,
}

#[derive(Subcommand)]
enum PricingSubcommands {
    /// Fetch the latest prices now, regardless of cache age
    Refresh,
    /// Show the price used for one or more models and where it comes from
    Show {
        /// Model names as they appear in logs (e.g. claude-opus-5-5)
        #[arg(required = true)]
        models: Vec<String>,
    },
}

#[derive(Args)]
struct WarpArgs {
    #[command(subcommand)]
    command: WarpSubcommands,
}

#[derive(Subcommand)]
enum WarpSubcommands {
    /// Setup WARP integration (captures auth token)
    Setup,
    /// Refresh expired WARP token
    Refresh,
    /// Check WARP token status
    Status,
}

#[derive(Subcommand)]
enum ConfigSubcommands {
    /// Create default configuration file
    Init {
        #[arg(long, default_value_t = false)]
        overwrite: bool,
    },
    /// Show current configuration
    Show,
    /// Set configuration value
    Set {
        /// Configuration key (api-token, auto-upload, number-comma, number-human, locale, decimal-places, health-display-style, notifications-enabled, notifications-wait-seconds, notifications-stale-minutes, notifications-sample-messages, slack-enabled, slack-webhook-url, slack-channel, slack-username, pricing-feed-enabled)
        key: String,
        /// Configuration value
        value: String,
    },
    /// Toggle Slack notifications on/off
    SlackToggle,
}

#[derive(Args)]
struct ConfigArgs {
    #[command(subcommand)]
    subcommand: ConfigSubcommands,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Load config file to get defaults
    let config = config::Config::load().unwrap_or(None).unwrap_or_default();

    // Create format options merging config defaults with CLI overrides
    let format_options = utils::NumberFormatOptions {
        use_comma: cli.number_comma || config.formatting.number_comma,
        use_human: cli.number_human || config.formatting.number_human,
        locale: cli
            .locale
            .unwrap_or_else(|| config.formatting.locale.clone()),
        decimal_places: cli
            .decimal_places
            .unwrap_or(config.formatting.decimal_places),
    };

    // Before anything publishes or reads the broker: this machine's name there.
    mqtt::set_name(config.mqtt.name.as_deref());
    plan_limits::set_plan_config(config.plans.clone());

    // Prices must be in place before any analyzer parses a message.
    price_feed::init(
        config.pricing.overrides.clone(),
        config.pricing.feed_enabled,
    );

    match cli.command {
        None => {
            // No subcommand - run default behavior
            run_default(format_options, config).await;
        }
        Some(Commands::Mqtt { forget: Some(host) }) => match mqtt::forget(&config.mqtt, &host).await {
            Ok(n) => println!("Cleared {n} retained topics for {host}."),
            Err(e) => {
                eprintln!("Could not clear {host}: {e:#}");
                std::process::exit(1);
            }
        },
        Some(Commands::Mqtt { forget: None }) => run_mqtt(config).await,
        Some(Commands::Mcp) => mcp_server::run(),
        Some(Commands::Quotas { json }) => print_quotas(json),
        Some(Commands::Upload) => match run_upload().await.context("Failed to run upload") {
            Ok(_) => {}
            Err(e) => {
                tui::show_upload_error(&format!("{e:#}"));
                std::process::exit(1);
            }
        },
        Some(Commands::Config(config_args)) => {
            handle_config_subcommand(config_args).await;
        }
        Some(Commands::Warp(warp_args)) => {
            handle_warp_subcommand(warp_args).await;
        }
        Some(Commands::Pricing(pricing_args)) => {
            handle_pricing_subcommand(pricing_args, config.pricing.feed_enabled).await;
        }
    }
}

/// `quotas`: every window and the spending ranking, as a table or JSON.
fn print_quotas(as_json: bool) {
    let status = match advisor::read_status() {
        Ok(status) => status,
        Err(e) => {
            eprintln!("{e:#}");
            std::process::exit(1);
        }
    };
    let now = chrono::Utc::now();
    let quotas = advisor::quotas_answer(&status, now);
    let spend = advisor::where_to_spend_answer(&status, now);
    if as_json {
        println!("{}", serde_json::to_string_pretty(&serde_json::json!({"quotas": quotas, "where_to_spend": spend})).unwrap_or_default());
        return;
    }
    if let Some(note) = quotas["stale"].as_str() {
        println!("⚠️  {note}\n");
    }
    println!(
        "{:<22} {:<24} {:<6} {:>5} {:>14} {:>9} {:>9} {:>9} {:>11}  {:<20} 5h now",
        "Tool", "Account", "Plan", "Used", "Left", "Need", "Now", "Runs out", "Locked out", "Resets"
    );
    let pct = |v: &serde_json::Value| v.as_f64().map_or("-".into(), |p| format!("{p:.0}%"));
    let tks = |v: &serde_json::Value| v.as_f64().filter(|t| *t > 0.0).map_or("-".into(), |t| format!("{t:.0} tk/s"));
    let text = |v: &serde_json::Value| v.as_str().unwrap_or("-").to_string();
    for row in quotas["plans"].as_array().into_iter().flatten() {
        let w = &row["plan"];
        let five = &row["five_hour_now"];
        let five = if five.is_null() {
            "-".to_string()
        } else {
            format!("{} · {}", pct(&five["used_percent"]), text(&five["resets_in"]))
        };
        let left = match (w["left_output_tokens"].as_f64(), w["left_dollars"].as_f64()) {
            (Some(t), Some(d)) => format!("{} · ${d:.0}", advisor::human_tokens(t)),
            _ => "-".into(),
        };
        println!(
            "{:<22} {:<24} {:<6} {:>5} {:>14} {:>9} {:>9} {:>9} {:>11}  {:<20} {five}",
            text(&w["tool"]),
            text(&w["account"]).replace(" (personal)", ""),
            text(&w["window"]),
            pct(&w["used_percent"]),
            left,
            tks(&w["need_tks"]),
            tks(&w["now_tks"]),
            text(&w["runs_out_in"]),
            text(&w["locked_out_for"]),
            format!(
                "{} ({})",
                text(&w["resets_in"]),
                w["resets_at"].as_str().map(|t| t.get(5..16).unwrap_or(t).replace('T', " ")).unwrap_or_default()
            ),
        );
    }
    println!("\nWhere to spend next:");
    for (i, pick) in spend["ranking"].as_array().into_iter().flatten().enumerate() {
        let mark = if pick["available"].as_bool() == Some(true) { "" } else { " (unavailable)" };
        println!("  {}. {}{mark}: {}", i + 1, pick["tool"].as_str().unwrap_or("?"), pick["reason"].as_str().unwrap_or(""));
    }
}

async fn handle_pricing_subcommand(args: PricingArgs, feed_enabled: bool) {
    match args.command {
        PricingSubcommands::Refresh => {
            if !feed_enabled {
                println!(
                    "⚠️  Pricing feed is disabled (pricing-feed-enabled = false); refreshing anyway."
                );
            }
            match price_feed::refresh().await {
                Ok(report) => {
                    println!(
                        "✅ Loaded prices for {} models from {}",
                        report.models,
                        price_feed::FEED_URL
                    );
                    for name in &report.rejected {
                        println!(
                            "⚠️  Ignored a >10x price change for {name}; kept the previous price"
                        );
                    }
                }
                Err(e) => {
                    eprintln!("❌ Could not refresh prices: {e:#}");
                    std::process::exit(1);
                }
            }
        }
        PricingSubcommands::Show { models } => {
            match price_feed::cache_age() {
                Some(age) => println!("Price feed cached {}h ago", age.as_secs() / 3600),
                None => println!("No price feed cached; run `splitrail pricing refresh`"),
            }
            for model in models {
                println!("{}", models::describe_price(&model));
            }
        }
    }
}

/// Bring prices up to date before parsing. With no cache at all, wait for the first fetch so the
/// first run is priced from the feed; otherwise refresh in the background and let the cached
/// prices serve until it lands.
async fn prepare_prices(feed_enabled: bool) {
    if !feed_enabled {
        return;
    }
    if price_feed::has_cache() {
        tokio::spawn(price_feed::refresh_if_stale(feed_enabled));
    } else {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            price_feed::refresh_if_stale(feed_enabled),
        )
        .await;
    }
}

async fn handle_warp_subcommand(args: WarpArgs) {
    match args.command {
        WarpSubcommands::Setup => {
            println!("🔧 WARP Setup");
            println!("This will capture your WARP auth token using proxy interception.");
            println!("\nRun: ./scripts/start_warp_proxy_quiet.sh");
            println!("Then use WARP normally, and stop with: ./scripts/stop_warp_proxy.sh");
            println!("\nToken will be saved to ~/.config/splitrail/warp_token");
        }
        WarpSubcommands::Refresh => {
            println!("🔄 Refreshing WARP token...");
            println!("Run: ./scripts/start_warp_proxy_quiet.sh");
            println!("Use WARP, then: ./scripts/stop_warp_proxy.sh");
        }
        WarpSubcommands::Status => {
            println!("📊 Checking WARP token status...");
            match warp::health::check_warp_token_health().await {
                warp::health::WarpTokenStatus::Valid => {
                    println!("✅ WARP token is valid");
                }
                warp::health::WarpTokenStatus::NotConfigured => {
                    println!("⚠️  WARP token not configured");
                    println!("   Run: splitrail warp setup");
                }
                warp::health::WarpTokenStatus::Expired => {
                    println!("❌ WARP token expired");
                    println!("   Run: splitrail warp refresh");
                }
                warp::health::WarpTokenStatus::Error(e) => {
                    println!("❌ Error checking token: {}", e);
                }
            }
        }
    }
}

fn create_analyzer_registry() -> AnalyzerRegistry {
    let mut registry = AnalyzerRegistry::new();

    // Register available analyzers. One Claude Code analyzer per account, so each account's
    // usage and limits are shown separately (BZL-64); a single account keeps the plain name.
    let claude_accounts = config::Config::load()
        .unwrap_or(None)
        .unwrap_or_default()
        .claude_code
        .resolved_accounts();
    let label_accounts = claude_accounts.len() > 1;
    for (label, dir) in claude_accounts {
        let label = label_accounts.then_some(label.as_str());
        registry.register(ClaudeCodeAnalyzer::for_account(label, dir));
    }
    registry.register(ClineAnalyzer::new());
    // registry.register(RooCodeAnalyzer::new()); // Temporarily disabled
    registry.register(KiloCodeAnalyzer::new());
    // registry.register(GeminiCliAnalyzer::new()); // Retired 2026-09: Google shut Gemini CLI down for
    // individual accounts on 2026-06-18 in favor of Antigravity CLI. See docs/GEMINI_CLI_DEPRECATION.md.
    registry.register(QwenCodeAnalyzer::new());
    registry.register(CodexCliAnalyzer::new());
    registry.register(DroidCliAnalyzer::new());
    // registry.register(AmazonQAnalyzer::new()); // Replaced by Kiro CLI
    registry.register(KiroCliAnalyzer::new());
    registry.register(OpenCodeAnalyzer::new()); // Ready when OpenCode adds local data storage
    registry.register(OpenCodeAnalyzer::for_flavor(analyzers::opencode::KILO_CLI));
    registry.register(WarpDevAnalyzer::new());
    registry.register(AntigravityAnalyzer::new()); // Fed by scripts/agy_statusline_capture.py
    registry.register(CursorAgentAnalyzer::new()); // Fed by scripts/cursor_hook_capture.py
    registry.register(OllamaAnalyzer::new()); // Fed by scripts/ollama_usage_proxy.py
    registry.register(CopilotCliAnalyzer::new());
    registry.register(PiAgentAnalyzer::new());
    registry.register(MtplxAnalyzer::new());
    // registry.register(CopilotAnalyzer::new()); // Temporarily disabled

    registry
}

async fn run_default(format_options: utils::NumberFormatOptions, config: config::Config) {
    prepare_prices(config.pricing.feed_enabled).await;
    let registry = create_analyzer_registry();

    // Create file watcher
    let file_watcher = match watcher::FileWatcher::new(&registry) {
        Ok(watcher) => watcher,
        Err(e) => {
            eprintln!("Error setting up file watcher: {e}");
            std::process::exit(1);
        }
    };

    // Create real-time stats manager
    // Droid's plan limits come only from Factory's API; polls when a Factory key is configured.
    // Started before the initial parse so the first reading lands while it runs.
    tokio::spawn(droid_limits::run());
    tokio::spawn(cursor_limits::run());
    tokio::spawn(warp_limits::run());
    tokio::spawn(kiro_limits::run());

    let mut stats_manager = match watcher::RealtimeStatsManager::new(registry).await {
        Ok(manager) => manager,
        Err(e) => {
            eprintln!("Error loading analyzer stats: {e}");
            std::process::exit(1);
        }
    };

    // Get the initial stats to check if we have data
    let initial_stats = stats_manager.get_stats_receiver().borrow().clone();

    // Create upload status for TUI
    let upload_status = Arc::new(Mutex::new(tui::UploadStatus::None));

    // Set upload status on stats manager for real-time upload tracking
    stats_manager.set_upload_status(upload_status.clone());

    // Start notification manager if enabled
    if let Some(manager) = notifications::NotificationManager::from_config(&config) {
        let stats_rx = stats_manager.get_stats_receiver();
        tokio::spawn(async move {
            if let Err(e) = manager.run(stats_rx).await {
                eprintln!("Notification task stopped: {e:#}");
            }
        });
    }

    tokio::spawn(mqtt::run(config.mqtt.clone(), stats_manager.get_stats_receiver()));

    // Check if auto-upload is enabled and start background upload
    if config.upload.auto_upload {
        if config.is_configured() {
            let upload_status_clone = upload_status.clone();
            tokio::spawn(async move {
                upload::perform_background_upload(
                    initial_stats,
                    Some(upload_status_clone),
                    Some(500),
                )
                .await;
            });
        } else {
            // Auto-upload is enabled but configuration is incomplete
            if let Ok(mut status) = upload_status.lock() {
                if config.is_api_token_missing() && config.is_server_url_missing() {
                    *status = tui::UploadStatus::MissingConfig;
                } else if config.is_api_token_missing() {
                    *status = tui::UploadStatus::MissingApiToken;
                } else if config.is_server_url_missing() {
                    *status = tui::UploadStatus::MissingServerUrl;
                } else {
                    // Shouldn't happen since is_configured() returned false
                    *status = tui::UploadStatus::MissingConfig;
                }
            }
        }
    }

    tokio::spawn(self_update::watch());

    // Other machines on the MQTT broker, as extra columns (the publisher and notifications
    // above see only this machine's stats).
    let tui_stats = match rollup::start(&config.mqtt, mqtt::host_slug()) {
        Some(remote) => rollup::merged_channel(stats_manager.get_stats_receiver(), remote),
        None => stats_manager.get_stats_receiver(),
    };

    // The menu bar's numbers (SwiftBar), from the same merged stats the TUI shows.
    tokio::spawn(status_file::run(tui_stats.clone(), config.goal.monthly_tks));

    // Start real-time TUI with file watcher
    if let Err(e) = tui::run_tui(
        tui_stats,
        &format_options,
        upload_status.clone(),
        file_watcher,
        stats_manager,
    ) {
        eprintln!("Error displaying TUI: {e}");
    }
    mqtt::shutdown().await;
    if self_update::restart_pending() {
        let error = self_update::exec_new_binary();
        eprintln!("Could not restart into the new version: {error}");
    }
}

/// `splitrail mqtt`: keep stats current from the file watcher and publish them, until Ctrl-C.
async fn run_mqtt(config: config::Config) {
    let Some(host) = config.mqtt.host.clone() else {
        eprintln!("No MQTT broker configured. Add to ~/.splitrail.toml:\n\n[mqtt]\nhost = \"<broker>\"\nusername = \"<user>\"\n\nwith the password in ~/.config/splitrail/mqtt_password");
        std::process::exit(1);
    };
    prepare_prices(config.pricing.feed_enabled).await;
    let registry = create_analyzer_registry();
    let file_watcher = match watcher::FileWatcher::new(&registry) {
        Ok(watcher) => watcher,
        Err(e) => {
            eprintln!("Error setting up file watcher: {e}");
            std::process::exit(1);
        }
    };
    tokio::spawn(droid_limits::run());
    tokio::spawn(cursor_limits::run());
    tokio::spawn(warp_limits::run());
    tokio::spawn(kiro_limits::run());
    let mut stats_manager = match watcher::RealtimeStatsManager::new(registry).await {
        Ok(manager) => manager,
        Err(e) => {
            eprintln!("Error loading analyzer stats: {e}");
            std::process::exit(1);
        }
    };
    tokio::spawn(mqtt::run(config.mqtt.clone(), stats_manager.get_stats_receiver()));
    // On a Mac, keep the menu bar's status file current too, other machines included. Linux
    // publishers have no menu bar and skip the roll-up subscription it needs.
    if cfg!(target_os = "macos") {
        let stats = match rollup::start(&config.mqtt, mqtt::host_slug()) {
            Some(remote) => rollup::merged_channel(stats_manager.get_stats_receiver(), remote),
            None => stats_manager.get_stats_receiver(),
        };
        tokio::spawn(status_file::run(stats, config.goal.monthly_tks));
    }
    println!("Publishing to {host} (Ctrl-C to stop)");

    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = tick.tick() => {}
        }
        while let Some(event) = file_watcher.try_recv() {
            if let Err(e) = stats_manager.handle_watcher_event(event).await {
                utils::log_error(format!("Error handling watcher event: {e:#}"));
            }
        }
        if let Err(e) = stats_manager.process_pending_batches().await {
            utils::log_error(format!("Error processing batched events: {e:#}"));
        }
        if let Err(e) = stats_manager.poll_codex_if_needed().await {
            utils::log_error(format!("Error polling Codex CLI: {e:#}"));
        }
    }
    mqtt::shutdown().await;
}

async fn run_upload() -> Result<()> {
    // Uploaded costs are durable, so wait for a stale feed to refresh rather than racing it.
    let feed_enabled = config::Config::load()
        .unwrap_or(None)
        .unwrap_or_default()
        .pricing
        .feed_enabled;
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        price_feed::refresh_if_stale(feed_enabled),
    )
    .await;
    let registry = create_analyzer_registry();
    let stats = registry.load_all_stats().await?;

    // Load config file to get formatting options
    let config_file = config::Config::load().unwrap_or(None).unwrap_or_default();
    let format_options = utils::NumberFormatOptions {
        use_comma: config_file.formatting.number_comma,
        use_human: config_file.formatting.number_human,
        locale: config_file.formatting.locale,
        decimal_places: config_file.formatting.decimal_places,
    };

    match config::Config::load() {
        Ok(Some(mut config)) if config.is_configured() => {
            // Clones only what is pending, not every retained message (BZL-14 phase 4).
            let messages =
                utils::messages_pending_upload(config.upload.last_date_uploaded, &stats);
            let progress_callback = tui::create_upload_progress_callback(&format_options);
            upload::upload_message_stats(&messages, &mut config, progress_callback)
                .await
                .context("Failed to upload messages")?;
            tui::show_upload_success(messages.len(), &format_options);
            Ok(())
        }
        Ok(Some(_)) => {
            eprintln!("Configuration incomplete");
            upload::show_upload_help();
            std::process::exit(1);
        }
        Ok(None) => {
            eprintln!("No configuration found");
            upload::show_upload_help();
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Config error: {e:#}");
            std::process::exit(1);
        }
    }
}

async fn handle_config_subcommand(config_args: ConfigArgs) {
    match config_args.subcommand {
        ConfigSubcommands::Init { overwrite } => {
            if let Err(e) = config::create_default_config(overwrite) {
                eprintln!("Error creating config: {e}");
                std::process::exit(1);
            }
        }
        ConfigSubcommands::Show => {
            if let Err(e) = config::show_config() {
                eprintln!("Error showing config: {e}");
                std::process::exit(1);
            }
        }
        ConfigSubcommands::Set { key, value } => {
            if let Err(e) = config::set_config_value(&key, &value) {
                eprintln!("Error setting config: {e}");
                std::process::exit(1);
            }
        }
        ConfigSubcommands::SlackToggle => {
            match config::Config::load() {
                Ok(Some(mut config)) => {
                    let _was_enabled = config.notifications.slack.enabled;
                    config.notifications.slack.enabled = !config.notifications.slack.enabled;

                    // Save the updated config
                    if let Err(e) = config.save(false) {
                        eprintln!("Failed to save config: {}", e);
                        std::process::exit(1);
                    } else {
                        let status = if config.notifications.slack.enabled {
                            "✅ Slack notifications enabled"
                        } else {
                            "❌ Slack notifications disabled"
                        };
                        println!("{}", status);
                    }
                }
                Ok(None) => {
                    println!(
                        "❌ No configuration found. Run 'splitrail config init' to create one."
                    );
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("❌ Failed to load configuration: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
