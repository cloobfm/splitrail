use anyhow::Result;
use notify::{RecursiveMode, Watcher};
use notify_types::event::{Event, EventKind};
use std::collections::HashSet;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::watch;

use crate::analyzer::{AnalyzerRegistry, AnalyzerWatchDir, sources_fingerprint};
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
    /// Boxed because the kind is chosen at runtime: the native backend normally, or a polling
    /// one when the OS refuses more watches (BZL-12).
    _watcher: Box<dyn Watcher + Send>,
    event_rx: Receiver<WatcherEvent>,
}

/// Whether this error is the OS refusing further watches.
///
/// On Linux `fs.inotify.max_user_watches` caps watches per user, and `notify` reports the
/// resulting ENOSPC as [`notify::ErrorKind::MaxFilesWatch`]. Every later watch fails too, so
/// without noticing this the dashboard would look live while seeing nothing (BZL-12).
fn is_watch_limit_reached(error: &notify::Error) -> bool {
    matches!(error.kind, notify::ErrorKind::MaxFilesWatch)
}

/// How often the polling fallback re-stats the watched trees. Long enough not to undo the parse
/// caches' CPU savings, short enough that the dashboard still feels live.
const POLL_INTERVAL: Duration = Duration::from_secs(5);

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

        // The handler is built per watcher, because a fallback needs its own copy.
        let make_handler = || {
            let event_tx = event_tx.clone();
            let dir_to_analyzer = dir_to_analyzer.clone();
            let debug_config = debug_config_for_handler.clone();
            move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    if let Err(e) =
                        handle_fs_event(event, &event_tx, &dir_to_analyzer, debug_config.as_ref())
                    {
                        let _ = event_tx
                            .send(WatcherEvent::Error(format!("Event handling error: {e}")));
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(WatcherEvent::Error(format!("Watch error: {e}")));
                }
            }
        };

        // The native backend first: FSEvents on macOS, inotify on Linux.
        let mut watcher: Box<dyn Watcher + Send> =
            Box::new(notify::recommended_watcher(make_handler())?);
        let mut exhausted = false;

        for dir in &watched_dirs {
            if let Err(e) = watcher.watch(dir, RecursiveMode::Recursive) {
                // Linux caps inotify watches per user (fs.inotify.max_user_watches). Watching a
                // tree the size of ~/.claude/projects can exhaust it, and every later watch then
                // fails too -- so the dashboard would sit there looking live while seeing nothing.
                if is_watch_limit_reached(&e) {
                    exhausted = true;
                    break;
                }
                crate::utils::report(format!(
                    "Warning: Could not watch directory {}: {}",
                    dir.display(),
                    e
                ));
            }
        }

        if exhausted {
            crate::utils::report(format!(
                "Warning: the OS file-watch limit was reached, so changes are polled every {}s                  instead of watched. Raise it with `sudo sysctl fs.inotify.max_user_watches=524288`                  (persist in /etc/sysctl.conf) for instant updates.",
                POLL_INTERVAL.as_secs()
            ));
            let config = notify::Config::default().with_poll_interval(POLL_INTERVAL);
            let mut poller = notify::PollWatcher::new(make_handler(), config)?;
            for dir in &watched_dirs {
                if let Err(e) = poller.watch(dir, RecursiveMode::Recursive) {
                    crate::utils::report(format!(
                        "Warning: Could not poll directory {}: {}",
                        dir.display(),
                        e
                    ));
                }
            }
            watcher = Box::new(poller);
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
    /// Fingerprint (path + size + mtime of every source file) of the Codex CLI data the last
    /// time it was loaded. The periodic poll only re-parses when this changes.
    last_codex_fingerprint: Option<u64>,
    /// Batch pending file events to process together
    pending_events: HashSet<String>,
    /// Time when first event in current batch was received
    batch_start_time: Option<Instant>,
    /// How long to wait for more events before processing batch
    batch_window: Duration,
    /// Analyzer reloads done, for tests and the CPU acceptance run.
    reloads: u64,
}

impl RealtimeStatsManager {
    pub async fn new(registry: AnalyzerRegistry) -> Result<Self> {
        // Initial stats load using registry method
        let initial_stats = registry.load_all_stats().await?;
        let (update_tx, update_rx) = watch::channel(initial_stats.clone());
        // Snapshot Codex's on-disk state now so the first poll does not redo the load we just did.
        let last_codex_fingerprint = Self::codex_fingerprint(&registry);

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
            poll_interval: Duration::from_secs(5), // Check Codex CLI for changes every 5 seconds
            last_codex_fingerprint,
            pending_events: HashSet::new(),
            batch_start_time: None,
            batch_window: Duration::from_millis(500), // Wait 500ms for more events
            reloads: 0,
        })
    }

    pub fn set_upload_status(&mut self, status: Arc<Mutex<UploadStatus>>) {
        self.upload_status = Some(status);
    }

    pub fn get_stats_receiver(&self) -> watch::Receiver<MultiAnalyzerStats> {
        self.update_rx.clone()
    }

    /// Path + size + mtime fingerprint of every Codex CLI source file, or None if Codex is not
    /// registered or discovery fails.
    fn codex_fingerprint(registry: &AnalyzerRegistry) -> Option<u64> {
        let analyzer = registry.get_analyzer_by_display_name("Codex CLI")?;
        let sources = analyzer.discover_data_sources().ok()?;
        Some(sources_fingerprint(&sources))
    }

    /// Safety net for Codex CLI, whose directory FSEvents has been unreliable for: every
    /// `poll_interval`, stat its source files and reload only if any of them changed. The stat
    /// pass costs a few milliseconds; the reload it used to do unconditionally cost a full parse
    /// of every Codex session on disk.
    pub async fn poll_codex_if_needed(&mut self) -> Result<()> {
        // Skip polling if disabled via env var
        if std::env::var("SPLITRAIL_DISABLE_POLLING").is_ok() {
            return Ok(());
        }

        let now = Instant::now();
        if now.duration_since(self.last_poll_time) < self.poll_interval {
            return Ok(());
        }
        self.last_poll_time = now;

        let current = Self::codex_fingerprint(&self.registry);
        if current.is_some() && current == self.last_codex_fingerprint {
            return Ok(()); // Nothing on disk changed; skip the re-parse.
        }

        // Queued with the file events rather than reloaded here, so a Codex write that both the
        // watcher and this poll notice in the same window costs one reload, not two.
        self.last_codex_fingerprint = current;
        self.queue_reload("Codex CLI".to_string());
        Ok(())
    }

    /// Ask for `analyzer_name` to be reloaded with the current batch.
    fn queue_reload(&mut self, analyzer_name: String) {
        self.pending_events.insert(analyzer_name);
        self.batch_start_time.get_or_insert_with(Instant::now);
    }

    pub async fn handle_watcher_event(&mut self, event: WatcherEvent) -> Result<()> {
        match event {
            WatcherEvent::DataChanged(analyzer_name) => self.queue_reload(analyzer_name),
            WatcherEvent::Error(err) => {
                crate::utils::report(format!("❌ Watcher error: {err}"));
            }
        }
        Ok(())
    }

    /// Check if batch window has expired and process pending events
    pub async fn process_pending_batches(&mut self) -> Result<()> {
        if let Some(batch_start) = self.batch_start_time {
            let now = Instant::now();
            if now.duration_since(batch_start) >= self.batch_window {
                let events_to_process: Vec<_> = self.pending_events.drain().collect();
                self.batch_start_time = None;

                for analyzer_name in events_to_process {
                    // Reloaded too recently: keep it pending for a later batch. Dropping it lost
                    // the last write of a burst until some unrelated write came along.
                    if let Some(last_reload) = self.last_reload_times.get(&analyzer_name)
                        && now.duration_since(*last_reload) < self.reload_debounce
                    {
                        self.queue_reload(analyzer_name);
                        continue;
                    }

                    self.last_reload_times.insert(analyzer_name.clone(), now);
                    if analyzer_name == "Codex CLI" {
                        // This reload covers whatever the poll would find next.
                        self.last_codex_fingerprint = Self::codex_fingerprint(&self.registry);
                    }

                    if let Err(e) = self.reload_analyzer_stats(&analyzer_name, true).await {
                        crate::utils::report(format!("❌ Error reloading {analyzer_name}: {e}"));
                    }
                    self.reloads += 1;
                }
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

        let mut new_stats = analyzer.get_stats().await?;
        crate::utils::retain_live_window(&mut new_stats, crate::utils::current_live_window_cutoff());
        let new_stats = Arc::new(new_stats);
        // Arc clones: the other analyzers' stats are shared, not copied.
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

        // Only the messages past the watermark are cloned, not every retained one (BZL-14
        // phase 4).
        let messages_to_upload = utils::messages_pending_upload(
            config.upload.last_date_uploaded,
            &self.current_stats,
        );

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

#[cfg(test)]
mod coalesce_tests {
    use super::*;
    use crate::analyzer::{Analyzer, DataSource};
    use crate::types::{AgenticCodingToolStats, ConversationMessage};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A "Codex CLI" whose single source file the test rewrites, counting full loads.
    struct FakeCodex {
        file: std::path::PathBuf,
        loads: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl Analyzer for FakeCodex {
        fn display_name(&self) -> &'static str {
            "Codex CLI"
        }
        fn get_data_glob_patterns(&self) -> Vec<String> {
            vec![]
        }
        fn discover_data_sources(&self) -> Result<Vec<DataSource>> {
            Ok(vec![DataSource { path: self.file.clone() }])
        }
        async fn parse_conversations(&self, _: Vec<DataSource>) -> Result<Vec<ConversationMessage>> {
            Ok(vec![])
        }
        async fn get_stats(&self) -> Result<AgenticCodingToolStats> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            Ok(AgenticCodingToolStats {
                daily_stats: Default::default(),
                num_conversations: 0,
                messages: vec![],
                analyzer_name: "Codex CLI".into(),
            })
        }
        fn is_available(&self) -> bool {
            true
        }
    }

    #[tokio::test]
    async fn a_file_event_and_the_poll_for_one_write_reload_once_and_nothing_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("rollout.jsonl");
        std::fs::write(&file, "a").unwrap();
        let loads = Arc::new(AtomicUsize::new(0));
        let mut registry = AnalyzerRegistry::new();
        registry.register(FakeCodex { file: file.clone(), loads: loads.clone() });
        let mut manager = RealtimeStatsManager::new(registry).await.unwrap();
        assert_eq!(loads.load(Ordering::SeqCst), 1, "initial load");
        manager.batch_window = Duration::ZERO;
        manager.poll_interval = Duration::ZERO;

        // One write, seen by both the watcher and the poll before the batch runs.
        std::fs::write(&file, "ab").unwrap();
        manager.handle_watcher_event(WatcherEvent::DataChanged("Codex CLI".into())).await.unwrap();
        manager.poll_codex_if_needed().await.unwrap();
        manager.process_pending_batches().await.unwrap();
        assert_eq!(manager.reloads, 1);
        // The reload refreshed the fingerprint, so the next poll finds nothing new.
        manager.poll_codex_if_needed().await.unwrap();
        manager.process_pending_batches().await.unwrap();
        assert_eq!(manager.reloads, 1);

        // A second write inside the debounce is deferred, not dropped.
        std::fs::write(&file, "abc").unwrap();
        manager.handle_watcher_event(WatcherEvent::DataChanged("Codex CLI".into())).await.unwrap();
        manager.process_pending_batches().await.unwrap();
        assert_eq!(manager.reloads, 1, "debounced");
        assert!(manager.pending_events.contains("Codex CLI"));
        manager.reload_debounce = Duration::ZERO;
        manager.process_pending_batches().await.unwrap();
        assert_eq!(manager.reloads, 2);
        assert_eq!(loads.load(Ordering::SeqCst), 3);
    }
}

#[cfg(test)]
mod watch_limit_tests {
    use super::is_watch_limit_reached;

    #[test]
    fn the_os_watch_limit_is_recognised() {
        let hit = notify::Error::new(notify::ErrorKind::MaxFilesWatch);
        assert!(
            is_watch_limit_reached(&hit),
            "ENOSPC from inotify must trigger the polling fallback"
        );
    }

    /// Anything else is a per-directory problem (a path that vanished, a permission error) and
    /// must not demote the whole watcher to polling.
    #[test]
    fn other_watch_errors_do_not_trigger_the_fallback() {
        let missing = notify::Error::new(notify::ErrorKind::PathNotFound);
        let io = notify::Error::new(notify::ErrorKind::Io(std::io::Error::other("nope")));
        assert!(!is_watch_limit_reached(&missing));
        assert!(!is_watch_limit_reached(&io));
    }
}
