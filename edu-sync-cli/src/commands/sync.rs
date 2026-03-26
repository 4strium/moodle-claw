//! Synchronize course content to local storage.

use std::{
    borrow::Cow,
    future::Future,
    io,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};

use dialoguer::{
    console::{self, Alignment},
    Confirm,
};
use edu_sync::{
    account::{Account, Token},
    config::{AccountConfig, Config},
    content::{Content, Download, FileDownload, SyncStatus},
};
use futures_util::{
    future,
    stream::{self, FuturesOrdered, FuturesUnordered},
    StreamExt, TryFutureExt,
};
use indicatif::{BinaryBytes, MultiProgress, ProgressBar, ProgressStyle};
use tokio::{
    task,
    time::{self, sleep},
};
use tracing::{info, trace};

use super::{
    courses::find_course,
    output::{print_error, OutputFormat},
};

/// Synchronize content from a course to local storage.
#[derive(Debug, clap::Parser)]
pub struct Command {
    /// Course name or ID to sync. If not provided, syncs all enabled courses.
    course: Option<String>,
    /// Destination directory (overrides config).
    #[arg(long, short)]
    dest: Option<PathBuf>,
    /// Bypass confirmation prompts.
    #[arg(long)]
    no_confirm: bool,
    /// Output format.
    #[arg(long, short, value_enum, default_value_t = OutputFormat::Markdown)]
    output: OutputFormat,
}

impl Command {
    pub async fn run(self) -> anyhow::Result<()> {
        let mut config = Config::read().await?;

        if !config.has_accounts() {
            print_error(
                "No accounts configured. Run `moodle configure` first.",
                self.output,
            );
            return Ok(());
        }

        // If a specific course is requested, enable it for sync
        if let Some(ref course_query) = self.course {
            let (course_id, _, _) = match find_course(&config, course_query) {
                Some(c) => c,
                None => {
                    print_error(
                        &format!("Course '{}' not found.", course_query),
                        self.output,
                    );
                    return Ok(());
                }
            };

            // Temporarily enable this course for sync
            for account in config.accounts.values_mut() {
                if let Some(course) = account.courses.0.get_mut(&course_id) {
                    course.sync = true;
                }
            }
        }

        if !config.has_active_courses() {
            print_error(
                "No courses enabled for sync. Use `moodle sync <course>` to sync a specific course.",
                self.output,
            );
            return Ok(());
        }

        let syncer = Syncer::from(config, self.dest.clone()).await;
        syncer.sync(self.no_confirm).await?;

        Ok(())
    }
}

struct Syncer {
    parallel_downloads: usize,
    outdated_courses: Vec<CourseStatus>,
}

impl Syncer {
    async fn from(config: Config, dest_override: Option<PathBuf>) -> Self {
        eprintln!("Requesting content databases...");
        let parallel_downloads = config.parallel_downloads;
        let outdated_courses = config
            .accounts
            .into_iter()
            .flat_map(|(_account_name, account_config)| {
                let AccountConfig {
                    path,
                    courses,
                    id,
                    token,
                    ..
                } = account_config;
                let base_path = dest_override.clone().unwrap_or(path);
                let account = Account::new(id, token);
                let account = Arc::new(account);
                courses
                    .0
                    .into_iter()
                    .rev()
                    .filter(|(_, course_config)| course_config.sync)
                    .map(move |(course_id, course_config)| {
                        let course_path =
                            base_path.join(course_config.name_as_path_component().as_ref());
                        (account.clone(), course_id, course_config.name, course_path)
                    })
            })
            .map(|(account, course_id, course_name, course_path)| {
                tokio::spawn(async move {
                    let fetch_status = |course_path, course_name| async {
                        account
                            .get_contents(course_id, course_path)
                            .and_then(|contents| async {
                                let status = CourseStatus::from_contents(
                                    contents,
                                    account.token(),
                                    course_name,
                                )
                                .await;
                                Ok(status)
                            })
                            .await
                    };

                    let account_id = account.id();
                    let mut status = fetch_status(course_path.clone(), course_name.clone()).await;
                    for _ in 0..4 {
                        match &status {
                            Ok(_) => break,
                            Err(err) if err.is_http() => {
                                sleep(Duration::from_millis(100)).await;
                                eprintln!(
                                    "Could not get contents for {course_name} from {account_id} \
                                     ({err}). Retrying."
                                );
                                status =
                                    fetch_status(course_path.clone(), course_name.clone()).await;
                            }
                            Err(_) => break,
                        }
                    }

                    match status {
                        Ok(ok) => Some(ok),
                        Err(err) => {
                            eprintln!(
                                "Could not get contents for {course_name} from {account_id} \
                                 ({err}). Giving up."
                            );
                            None
                        }
                    }
                })
            })
            .collect::<FuturesOrdered<_>>()
            .filter_map(|res| async move { res.inspect_err(|err| eprintln!("{err}")).ok() })
            .filter_map(|res| async move { res })
            .filter(|course_status| future::ready(!course_status.downloads.is_empty()))
            .collect::<Vec<_>>()
            .await;
        Self {
            parallel_downloads,
            outdated_courses,
        }
    }

    async fn sync(self, no_confirm: bool) -> anyhow::Result<()> {
        if self.outdated_courses.is_empty() {
            eprintln!("All resources are up to date.");
        } else {
            eprintln!();

            let size_width = 9;
            let pad_course_name = |course_name| {
                let width = 80 - size_width - 4 - 19;
                console::pad_str(course_name, width, Alignment::Left, Some("..."))
            };
            let pad_size = |size| {
                let message = if size > 0 {
                    Cow::from(BinaryBytes(size).to_string())
                } else {
                    Cow::from("N/A")
                };
                console::pad_str(message.as_ref(), size_width, Alignment::Right, None).into_owned()
            };

            let (count, size) = self
                .outdated_courses
                .iter()
                .map(|course| {
                    let count = course.downloads.len();
                    let size = course.downloads.iter().map(Download::size).sum();
                    let name = &course.name;
                    (count, size, name)
                })
                .inspect(|(count, size, name)| {
                    eprintln!(
                        "{} {:>4} items, totalling {}",
                        pad_course_name(name),
                        count,
                        pad_size(*size)
                    );
                })
                .map(|(count, size, _name)| (count, size))
                .reduce(|(count_a, size_a), (count_b, size_b)| (count_a + count_b, size_a + size_b))
                .unwrap();

            let size_display = if size > 0 {
                Cow::from(BinaryBytes(size).to_string())
            } else {
                Cow::from("N/A")
            };

            eprintln!();
            eprintln!("Total: {} items, totalling {}", count, size_display);
            eprintln!();

            let proceed = no_confirm
                || task::spawn_blocking(|| {
                    Confirm::new()
                        .with_prompt("Proceed with synchronization?")
                        .default(true)
                        .interact()
                })
                .await??;

            if proceed {
                eprintln!("Downloading missing files...");
                self.download().await?;
            }
        }

        Ok(())
    }

    async fn download(self) -> io::Result<()> {
        let multi_progress = Arc::new(MultiProgress::new());
        let content_progress_style = ProgressStyle::default_bar()
            .template("[{pos}/{len}] {wide_msg}")
            .unwrap();
        let size_progress_style = ProgressStyle::default_bar()
            .template(
                "└──── {binary_bytes:>9} / {binary_total_bytes:>9} [{bar:25}] \
                 {binary_bytes_per_sec:>11} in {elapsed:>3} ETA: {eta:>3}",
            )
            .unwrap()
            .progress_chars("=> ");

        let multi_progress_clone = multi_progress.clone();
        let download_tasks = self
            .outdated_courses
            .into_iter()
            .map(
                |CourseStatus {
                     token,
                     name,
                     downloads,
                 }| {
                    let multi_progress = multi_progress_clone.clone();
                    let content_progress_style = content_progress_style.clone();
                    let size_progress_style = size_progress_style.clone();
                    let content_progress = multi_progress.add(
                        ProgressBar::new(0)
                            .with_style(content_progress_style)
                            .with_message(name),
                    );
                    let size_progress =
                        multi_progress.add(ProgressBar::new(0).with_style(size_progress_style));
                    tokio::spawn(async move {
                        CourseDownload {
                            downloads,
                            token,
                            content_progress,
                            size_progress,
                        }
                        .run()
                        .await
                    })
                },
            )
            .collect::<FuturesUnordered<_>>();

        let total_bar = multi_progress.add(
            ProgressBar::new(0).with_style(
                ProgressStyle::default_bar()
                    .template(
                        "Total {binary_bytes:>9} / {binary_total_bytes:>9} [{bar:25}] \
                         {binary_bytes_per_sec:>11} in {elapsed:>3} ETA: {eta:>3}",
                    )
                    .unwrap()
                    .progress_chars("=> "),
            ),
        );

        let (file_downloads, content_downloads, size_progress, content_progress, size) =
            download_tasks
                .filter_map(|res| future::ready(res.map_err(|err| eprintln!("{}", err)).ok()))
                .filter_map(|res| future::ready(res.map_err(|err| eprintln!("{}", err)).ok()))
                .fold(
                    (Vec::new(), Vec::new(), Vec::new(), Vec::new(), 0),
                    |(
                        mut file_downloads,
                        mut content_downloads,
                        mut size_progress,
                        mut content_progress,
                        size,
                    ),
                     mut download| async move {
                        file_downloads.append(&mut download.file_downloads);
                        content_downloads.append(&mut download.content_downloads);
                        size_progress.push((download.download_progresses, download.size_progress));
                        content_progress.push(download.content_progress);
                        (
                            file_downloads,
                            content_downloads,
                            size_progress,
                            content_progress,
                            size + download.size,
                        )
                    },
                )
                .await;

        total_bar.set_length(size);

        let size_progresses = size_progress
            .iter()
            .map(|(_, bar)| bar)
            .cloned()
            .collect::<Vec<_>>();

        let file_downloads = stream::iter(file_downloads)
            .map(tokio::spawn)
            .buffer_unordered(self.parallel_downloads)
            .collect::<Vec<_>>();

        let total_bar_clone = total_bar.clone();
        let size_task = tokio::spawn(async move {
            let mut timer = time::interval(Duration::from_millis(200));
            loop {
                let mut total = 0;
                for (progresses, size_progress) in &size_progress {
                    let progress = progresses
                        .iter()
                        .map(|progress| progress.load(Ordering::Relaxed))
                        .sum();
                    size_progress.set_position(progress);
                    total += progress;
                }
                total_bar_clone.set_position(total);
                timer.tick().await;
            }
        });

        let content_downloads = content_downloads
            .into_iter()
            .map(tokio::spawn)
            .collect::<Vec<_>>();
        let file_downloads = file_downloads.await;
        for content_download in content_downloads {
            content_download.await?;
        }

        size_task.abort();
        for size_progress in size_progresses {
            size_progress.finish();
        }
        for content_progress in content_progress {
            content_progress.finish();
        }
        total_bar.finish();

        for file_download in file_downloads {
            file_download??;
        }

        Ok(())
    }
}

struct CourseStatus {
    token: Token,
    name: String,
    downloads: Vec<Download>,
}

impl CourseStatus {
    async fn from_contents(
        contents: impl Iterator<Item = Content> + Send,
        token: Token,
        name: String,
    ) -> Self {
        let downloads = contents
            .map(|content| {
                tokio::spawn(async move {
                    match content.sync().await {
                        SyncStatus::Downloadable(download) => Some(download),
                        SyncStatus::NotSupported(content_type, path) => {
                            info!(
                                "Not supported: ContentType::{:?} at {}",
                                content_type,
                                path.display()
                            );
                            None
                        }
                        SyncStatus::UpToDate(path) => {
                            trace!("Up to date: {}", path.display());
                            None
                        }
                    }
                })
            })
            .collect::<FuturesUnordered<_>>()
            .filter_map(|res| future::ready(res.map_err(|err| eprintln!("{}", err)).ok()))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();
        Self {
            token,
            name,
            downloads,
        }
    }
}

struct CourseDownload {
    downloads: Vec<Download>,
    token: Token,
    content_progress: ProgressBar,
    size_progress: ProgressBar,
}

struct CourseDownloads<F, C> {
    file_downloads: Vec<F>,
    content_downloads: Vec<C>,
    download_progresses: Vec<Arc<AtomicU64>>,
    size_progress: ProgressBar,
    size: u64,
    content_progress: ProgressBar,
}

impl CourseDownload {
    async fn run(
        self,
    ) -> io::Result<CourseDownloads<impl Future<Output = io::Result<()>>, impl Future<Output = ()>>>
    {
        let Self {
            downloads,
            token,
            content_progress,
            size_progress,
        } = self;

        content_progress.set_length(downloads.len() as u64);

        let (file_downloads, content_downloads) = downloads
            .into_iter()
            .partition::<Vec<_>, _>(|download| matches!(download, Download::File(_)));

        let file_downloads = file_downloads
            .into_iter()
            .map(|file_download| match file_download {
                Download::File(file_download) => file_download,
                _ => unreachable!(),
            })
            .collect::<Vec<FileDownload>>();

        let download_size = file_downloads.iter().map(FileDownload::size).sum();
        size_progress.set_length(download_size);

        let progresses = file_downloads
            .iter()
            .map(|_| Arc::new(AtomicU64::new(0)))
            .collect::<Vec<_>>();
        let content_progress_clone = content_progress.clone();
        let file_downloads = file_downloads
            .into_iter()
            .zip(progresses.iter().cloned())
            .map(|(mut file_download, progress)| {
                let content_progress = content_progress_clone.clone();
                async move {
                    file_download
                        .run(&token, |val| progress.store(val, Ordering::Relaxed))
                        .await
                        .map(|()| {
                            content_progress.inc(1);
                            let path = file_download.path().display();
                            content_progress.println(path.to_string());
                        })
                        .inspect_err(|err| {
                            let path = file_download.path().display();
                            content_progress
                                .println(format!("error while downloading {path}: {err}"));
                        })
                }
            })
            .collect::<Vec<_>>();

        let content_progress_clone = content_progress.clone();
        let content_downloads = content_downloads
            .into_iter()
            .map(|download| {
                let content_progress = content_progress_clone.clone();
                async move {
                    match download {
                        Download::File(_) => unreachable!(),
                        Download::Url(mut url_download) => {
                            url_download.run().await.unwrap();
                            content_progress.inc(1);
                            let path = url_download.path().display().to_string();
                            content_progress.println(path);
                        }
                        Download::Content(mut content_download) => {
                            content_download.run().await.unwrap();
                            content_progress.inc(1);
                            let path = content_download.path().display().to_string();
                            content_progress.println(path);
                        }
                    }
                }
            })
            .collect::<Vec<_>>();

        Ok(CourseDownloads {
            file_downloads,
            content_downloads,
            download_progresses: progresses,
            size_progress,
            size: download_size,
            content_progress,
        })
    }
}
