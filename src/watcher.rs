use anyhow::Result;
use notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher};
use notify_types::event::{Event, EventKind};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

use crate::analyzer::{AnalyzerRegistry, AnalyzerWatchDir};
use crate::config::Config;
use crate::tui::UploadStatus;
use crate::types::MultiAnalyzerStats;
use crate::upload;

#[derive(Debug, Clone)]
pub enum WatcherEvent {
    DataChanged(String), // analyzer name
    Error(String),
}

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
    event_rx: Receiver<WatcherEvent>,
}

#[derive(Clone)]
struct WatchDebugConfig {
    analyzer_filter: Option<String>, // lowercase substring to match, None = log all
}

impl WatchDebugConfig {
    fn from_env() -> Option<Self> {
        match std::env::var("SPLITRAIL_DEBUG_WATCHERS") {
            Ok(val) => {
                let trimmed = val.trim();
                if trimmed.is_empty()
                    || trimmed == "1"
                    || trimmed.eq_ignore_ascii_case("true")
                    || trimmed.eq_ignore_ascii_case("all")
                {
                    Some(Self {
                        analyzer_filter: None,
                    })
                } else {
                    Some(Self {
                        analyzer_filter: Some(trimmed.to_lowercase()),
                    })
                }
            }
            Err(_) => None,
        }
    }

    fn matches(&self, analyzer_name: &str) -> bool {
        match &self.analyzer_filter {
            Some(filter) => analyzer_name.to_lowercase().contains(filter),
            None => true,
        }
    }
}

impl FileWatcher {
    pub fn new(registry: &AnalyzerRegistry) -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel();

        // Get directory to analyzer mapping from registry
        let dir_to_analyzer = registry.get_directory_to_analyzer_mapping();
        let debug_config = WatchDebugConfig::from_env();
        if let Some(config) = &debug_config {
            eprintln!(
                "[watch] Debug logging enabled for analyzer filter: {}",
                config
                    .analyzer_filter
                    .as_deref()
                    .unwrap_or("<all analyzers>")
            );
            for entry in &dir_to_analyzer {
                if config.matches(&entry.analyzer_name) {
                    eprintln!(
                        "[watch] {} -> match_dir={} watch_dir={} pattern={}",
                        entry.analyzer_name,
                        entry.match_dir.display(),
                        entry.watch_dir.display(),
                        entry.pattern.as_str()
                    );
                }
            }
        }

        let debug_config_for_handler = debug_config.clone();
        let watched_dirs: HashSet<_> = dir_to_analyzer
            .iter()
            .map(|entry| entry.watch_dir.clone())
            .collect();

        // Use RecommendedWatcher (FSEvents on macOS) with default config
        let mut watcher =
            notify::recommended_watcher(move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    if let Err(e) = handle_fs_event(
                        event,
                        &event_tx,
                        &dir_to_analyzer,
                        debug_config_for_handler.as_ref(),
                    ) {
                        let _ = event_tx
                            .send(WatcherEvent::Error(format!("Event handling error: {e}")));
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(WatcherEvent::Error(format!("Watch error: {e}")));
                }
            })?;

        // Start watching all directories
        for dir in &watched_dirs {
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                eprintln!(
                    "Warning: Could not watch directory {}: {}",
                    dir.display(),
                    e
                );
            }
        }

        Ok(Self {
            _watcher: watcher,
            event_rx,
        })
    }

    pub fn try_recv(&self) -> Option<WatcherEvent> {
        self.event_rx.try_recv().ok()
    }
}

fn handle_fs_event(
    event: Event,
    tx: &Sender<WatcherEvent>,
    dir_to_analyzer: &[AnalyzerWatchDir],
    debug_config: Option<&WatchDebugConfig>,
) -> Result<()> {
    // Only care about create, write, and remove events
    match event.kind {
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
            let mut notified_analyzers = HashSet::new();
            for path in &event.paths {
                for entry in dir_to_analyzer {
                    if !path.starts_with(&entry.watch_dir) {
                        continue;
                    }
                    if path.starts_with(&entry.match_dir) || entry.pattern.matches_path(path) {
                        if notified_analyzers.insert(entry.analyzer_name.clone()) {
                            let _ = tx.send(WatcherEvent::DataChanged(entry.analyzer_name.clone()));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

pub struct RealtimeStatsManager {
    registry: AnalyzerRegistry,
    current_stats: MultiAnalyzerStats,
    update_tx: watch::Sender<MultiAnalyzerStats>,
    update_rx: watch::Receiver<MultiAnalyzerStats>,
    last_upload_time: Option<Instant>,
    upload_debounce: Duration,
    upload_status: Option<Arc<Mutex<UploadStatus>>>,
    upload_in_progress: Arc<Mutex<bool>>,
    pending_upload: Arc<Mutex<bool>>,
    last_reload_times: std::collections::HashMap<String, Instant>,
    reload_debounce: Duration,
    last_poll_time: Instant,
    poll_interval: Duration,
}

impl RealtimeStatsManager {
    pub async fn new(registry: AnalyzerRegistry) -> Result<Self> {
        // Initial stats load using registry method
        let initial_stats = registry.load_all_stats().await?;
        let (update_tx, update_rx) = watch::channel(initial_stats.clone());

        Ok(Self {
            registry,
            current_stats: initial_stats,
            update_tx,
            update_rx,
            last_upload_time: None,
            upload_debounce: Duration::from_secs(3),
            upload_status: None,
            upload_in_progress: Arc::new(Mutex::new(false)),
            pending_upload: Arc::new(Mutex::new(false)),
            last_reload_times: std::collections::HashMap::new(),
            reload_debounce: Duration::from_secs(2),
            last_poll_time: Instant::now(),
            poll_interval: Duration::from_secs(5), // Poll Codex CLI every 5 seconds
        })
    }

    pub fn set_upload_status(&mut self, status: Arc<Mutex<UploadStatus>>) {
        self.upload_status = Some(status);
    }

    pub fn get_stats_receiver(&self) -> watch::Receiver<MultiAnalyzerStats> {
        self.update_rx.clone()
    }

    /// Check if enough time has passed since last poll and reload Codex CLI if needed
    pub async fn poll_codex_if_needed(&mut self) -> Result<()> {
        // Skip polling if disabled via env var
        if std::env::var("SPLITRAIL_DISABLE_POLLING").is_ok() {
            return Ok(());
        }

        let now = Instant::now();
        if now.duration_since(self.last_poll_time) >= self.poll_interval {
            self.last_poll_time = now;

            // Reload Codex CLI specifically
            if let Some(analyzer) = self.registry.get_analyzer_by_display_name("Codex CLI") {
                match analyzer.get_stats().await {
                    Ok(new_stats) => {
                        let mut updated_analyzer_stats = self.current_stats.analyzer_stats.clone();

                        if let Some(pos) = updated_analyzer_stats
                            .iter()
                            .position(|s| s.analyzer_name == "Codex CLI")
                        {
                            updated_analyzer_stats[pos] = new_stats;
                        } else {
                            updated_analyzer_stats.push(new_stats);
                        }

                        self.current_stats = MultiAnalyzerStats {
                            analyzer_stats: updated_analyzer_stats,
                        };

                        let _ = self.update_tx.send(self.current_stats.clone());
                    }
                    Err(_) => {} // Silently ignore polling errors
                }
            }
        }
        Ok(())
    }

    pub async fn handle_watcher_event(&mut self, event: WatcherEvent) -> Result<()> {
        match event {
            WatcherEvent::DataChanged(analyzer_name) => {
                // Check debounce - only reload if enough time has passed since last reload
                let now = Instant::now();
                if let Some(last_reload) = self.last_reload_times.get(&analyzer_name) {
                    if now.duration_since(*last_reload) < self.reload_debounce {
                        // Skip this reload, too soon
                        return Ok(());
                    }
                }

                // Update last reload time
                self.last_reload_times.insert(analyzer_name.clone(), now);

                // Reload data for the specific analyzer
                if let Some(analyzer) = self.registry.get_analyzer_by_display_name(&analyzer_name) {
                    match analyzer.get_stats().await {
                        Ok(new_stats) => {
                            // Update the stats for this analyzer
                            let mut updated_analyzer_stats =
                                self.current_stats.analyzer_stats.clone();

                            // Find and replace the stats for this analyzer
                            if let Some(pos) = updated_analyzer_stats
                                .iter()
                                .position(|s| s.analyzer_name == analyzer_name)
                            {
                                updated_analyzer_stats[pos] = new_stats;
                            } else {
                                // New analyzer data
                                updated_analyzer_stats.push(new_stats);
                            }

                            self.current_stats = MultiAnalyzerStats {
                                analyzer_stats: updated_analyzer_stats,
                            };

                            // Send the update
                            let _ = self.update_tx.send(self.current_stats.clone());

                            // Trigger auto-upload if enabled and debounce time has passed
                            self.trigger_auto_upload_if_enabled().await;
                        }
                        Err(e) => {
                            eprintln!("❌ Error reloading {analyzer_name}: {e}");
                        }
                    }
                }
            }
            WatcherEvent::Error(err) => {
                eprintln!("❌ Watcher error: {err}");
            }
        }
        Ok(())
    }

    async fn trigger_auto_upload_if_enabled(&mut self) {
        // Check if auto-upload is enabled
        let _config = match Config::load() {
            Ok(Some(cfg)) if cfg.upload.auto_upload && cfg.is_configured() => cfg,
            _ => return, // Auto-upload not enabled or config not available
        };

        // Check if an upload is already in progress
        if let Ok(in_progress) = self.upload_in_progress.lock()
            && *in_progress
        {
            // Mark that we have pending changes to upload
            if let Ok(mut pending) = self.pending_upload.lock() {
                *pending = true;
            }
            return;
        }

        // Check debounce timing
        let now = Instant::now();
        if let Some(last_time) = self.last_upload_time
            && now.duration_since(last_time) < self.upload_debounce
        {
            // Schedule a delayed upload
            let remaining_wait = self.upload_debounce - now.duration_since(last_time);
            let stats = self.current_stats.clone();
            let upload_status = self.upload_status.clone();
            let upload_in_progress = self.upload_in_progress.clone();
            let pending_upload = self.pending_upload.clone();

            tokio::spawn(async move {
                tokio::time::sleep(remaining_wait).await;

                // Check if we should still upload
                let should_upload = if let Ok(mut pending) = pending_upload.lock() {
                    let was_pending = *pending;
                    *pending = false;
                    was_pending
                } else {
                    true
                };

                if should_upload {
                    // Mark upload as in progress
                    if let Ok(mut in_progress) = upload_in_progress.lock() {
                        *in_progress = true;
                    }

                    upload::perform_background_upload(stats, upload_status, None).await;

                    // Mark upload as complete
                    if let Ok(mut in_progress) = upload_in_progress.lock() {
                        *in_progress = false;
                    }
                }
            });

            // Mark that we have a pending upload scheduled
            if let Ok(mut pending) = self.pending_upload.lock() {
                *pending = true;
            }
            return;
        }

        self.last_upload_time = Some(now);

        // Mark upload as in progress
        if let Ok(mut in_progress) = self.upload_in_progress.lock() {
            *in_progress = true;
        }

        // Clone necessary data for the async upload task
        let stats = self.current_stats.clone();
        let upload_status = self.upload_status.clone();
        let upload_in_progress = self.upload_in_progress.clone();
        let pending_upload = self.pending_upload.clone();

        // Spawn background upload task
        tokio::spawn(async move {
            upload::perform_background_upload(stats.clone(), upload_status.clone(), None).await;

            // Mark upload as complete
            if let Ok(mut in_progress) = upload_in_progress.lock() {
                *in_progress = false;
            }

            // Check if we need to upload again due to changes during the upload
            let should_upload_again = if let Ok(mut pending) = pending_upload.lock() {
                let was_pending = *pending;
                *pending = false;
                was_pending
            } else {
                false
            };

            if should_upload_again {
                // Wait a short time before uploading again
                tokio::time::sleep(Duration::from_secs(1)).await;
                upload::perform_background_upload(stats, upload_status, None).await;
            }
        });
    }
}
