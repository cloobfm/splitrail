use anyhow::{Context, Result};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
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
    _debug_config: Option<&WatchDebugConfig>,
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
    /// Batch pending file events to process together
    pending_events: HashSet<String>,
    /// Time when first event in current batch was received
    batch_start_time: Option<Instant>,
    /// How long to wait for more events before processing batch
    batch_window: Duration,
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
            pending_events: HashSet::new(),
            batch_start_time: None,
            batch_window: Duration::from_millis(500), // Wait 500ms for more events
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
            let _ = self.reload_analyzer_stats("Codex CLI", false).await;
        }
        Ok(())
    }

    pub async fn handle_watcher_event(&mut self, event: WatcherEvent) -> Result<()> {
        match event {
            WatcherEvent::DataChanged(analyzer_name) => {
                // Add to pending batch
                self.pending_events.insert(analyzer_name);

                // Start batch timer if this is the first event
                if self.batch_start_time.is_none() {
                    self.batch_start_time = Some(Instant::now());
                }
            }
            WatcherEvent::Error(err) => {
                eprintln!("❌ Watcher error: {err}");
            }
        }
        Ok(())
    }

    /// Check if batch window has expired and process pending events
    pub async fn process_pending_batches(&mut self) -> Result<()> {
        if let Some(batch_start) = self.batch_start_time {
            let now = Instant::now();
            if now.duration_since(batch_start) >= self.batch_window {
                // Process all pending events
                let events_to_process: Vec<_> = self.pending_events.drain().collect();

                for analyzer_name in events_to_process {
                    // Check debounce
                    if let Some(last_reload) = self.last_reload_times.get(&analyzer_name) {
                        if now.duration_since(*last_reload) < self.reload_debounce {
                            continue; // Skip, too soon
                        }
                    }

                    // Update last reload time
                    self.last_reload_times.insert(analyzer_name.clone(), now);

                    if let Err(e) = self.reload_analyzer_stats(&analyzer_name, true).await {
                        eprintln!("❌ Error reloading {analyzer_name}: {e}");
                    }
                }

                // Reset batch timer
                self.batch_start_time = None;
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

    async fn reload_analyzer_stats(
        &mut self,
        analyzer_name: &str,
        trigger_upload: bool,
    ) -> Result<()> {
        let Some(analyzer) = self.registry.get_analyzer_by_display_name(analyzer_name) else {
            return Ok(());
        };

        let new_stats = analyzer.get_stats().await?;
        let mut updated_analyzer_stats = self.current_stats.analyzer_stats.clone();

        if let Some(pos) = updated_analyzer_stats
            .iter()
            .position(|s| s.analyzer_name == analyzer_name)
        {
            updated_analyzer_stats[pos] = new_stats;
        } else {
            updated_analyzer_stats.push(new_stats);
        }

        self.current_stats = MultiAnalyzerStats {
            analyzer_stats: updated_analyzer_stats,
        };

        let _ = self.update_tx.send(self.current_stats.clone());

        if trigger_upload {
            self.trigger_auto_upload_if_enabled().await;
        }

        Ok(())
    }

    /// Trigger a manual upload regardless of auto-upload settings
    pub async fn trigger_manual_upload(&mut self) -> Result<()> {
        use crate::config::Config;
        use crate::upload;
        use crate::utils;

        // Check if an upload is already in progress
        if let Ok(in_progress) = self.upload_in_progress.lock()
            && *in_progress
        {
            return Ok(()); // Upload already in progress
        }

        // Check config is available
        let mut config = match Config::load() {
            Ok(Some(cfg)) if cfg.is_configured() => cfg,
            _ => return Ok(()), // Config not available or incomplete
        };

        // Collect all messages from current stats
        let mut messages = vec![];
        for analyzer_stats in &self.current_stats.analyzer_stats {
            messages.extend(analyzer_stats.messages.clone());
        }

        // Filter messages to upload only those not yet uploaded
        let messages_to_upload = utils::get_messages_later_than(config.upload.last_date_uploaded, messages)
            .await
            .context("Failed to get messages later than last saved date")?;

        if messages_to_upload.is_empty() {
            return Ok(()); // Nothing to upload
        }

        // Set upload status
        if let Some(upload_status) = &self.upload_status {
            if let Ok(mut status) = upload_status.lock() {
                *status = UploadStatus::Uploading {
                    current: 0,
                    total: messages_to_upload.len(),
                    dots: 0,
                };
            }
        }

        // Mark upload as in progress
        if let Ok(mut in_progress) = self.upload_in_progress.lock() {
            *in_progress = true;
        }

        let _messages_len = messages_to_upload.len();
        let upload_status = self.upload_status.clone();
        let upload_in_progress = self.upload_in_progress.clone();

        // Spawn upload task
        tokio::spawn(async move {
            let result = upload::upload_message_stats(&messages_to_upload, &mut config, |_current, _total| {
                if let Some(status) = &upload_status {
                    if let Ok(mut status_guard) = status.lock() {
                        if let UploadStatus::Uploading { dots, .. } = &mut *status_guard {
                            *dots = (*dots + 1) % 4;
                        }
                    }
                }
            }).await;

            // Update status based on result
            if let Some(status) = &upload_status {
                if let Ok(mut status_guard) = status.lock() {
                    match result {
                        Ok(_) => {
                            *status_guard = UploadStatus::Uploaded;
                        }
                        Err(e) => {
                            *status_guard = UploadStatus::Failed(e.to_string());
                        }
                    }
                }
            }

            // Mark upload as complete
            if let Ok(mut in_progress) = upload_in_progress.lock() {
                *in_progress = false;
            }
        });

        self.last_upload_time = Some(Instant::now());
        Ok(())
    }
}
