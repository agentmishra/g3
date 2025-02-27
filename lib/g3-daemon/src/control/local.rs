/*
 * Copyright 2023 ByteDance and/or its affiliates.
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::fs::DirBuilder;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::anyhow;
use futures_util::future::{AbortHandle, Abortable};
use log::{debug, warn};
use tokio::io::{AsyncRead, AsyncWrite, BufReader};
#[cfg(unix)]
use tokio::net::UnixListener;

use super::{CtlProtoCtx, CtlProtoType, LocalControllerConfig};

static UNIQUE_CONTROLLER_ABORT_HANDLER: Mutex<Option<AbortHandle>> = Mutex::new(None);
static DAEMON_CONTROLLER_ABORT_HANDLER: Mutex<Option<AbortHandle>> = Mutex::new(None);

fn ctl_handle<R, W>(r: R, w: W)
where
    R: AsyncRead + Send + Unpin + 'static,
    W: AsyncWrite + Send + Unpin + 'static,
{
    let ctx = CtlProtoCtx::new(
        BufReader::new(r),
        w,
        LocalControllerConfig::get_general(),
        CtlProtoType::Text,
    );
    tokio::spawn(async move {
        if let Err(e) = ctx.run().await {
            warn!("error handle client: {e}");
        }
    });
}

#[cfg(unix)]
pub struct LocalController {
    listen_path: PathBuf,
    listener: UnixListener,
}

#[cfg(unix)]
impl LocalController {
    fn new(listen_path: PathBuf) -> io::Result<Self> {
        let listener = UnixListener::bind(&listen_path)?;
        Ok(LocalController {
            listen_path,
            listener,
        })
    }

    pub fn listen_path(&self) -> PathBuf {
        self.listen_path.clone()
    }

    pub fn create_unique(daemon_group: &str) -> anyhow::Result<Self> {
        let socket_name = format!("{daemon_group}_{}.sock", std::process::id());
        let mut listen_path = crate::opts::control_dir();
        listen_path.push(Path::new(&socket_name));
        check_then_finalize_path(&listen_path)?;

        debug!("setting up unique controller {}", listen_path.display());
        let controller = LocalController::new(listen_path)?;
        debug!("unique controller created");
        Ok(controller)
    }

    pub fn create_daemon(daemon_group: &str) -> anyhow::Result<Self> {
        let socket_name = if daemon_group.is_empty() {
            "_.sock".to_string()
        } else {
            format!("{daemon_group}.sock")
        };
        let mut listen_path = crate::opts::control_dir();
        listen_path.push(Path::new(&socket_name));
        check_then_finalize_path(&listen_path)?;

        debug!("setting up daemon controller {}", listen_path.display());
        let controller = LocalController::new(listen_path)?;
        debug!("daemon controller created");
        Ok(controller)
    }

    async fn into_running(self) {
        loop {
            let result = self.listener.accept().await;
            match result {
                Ok((stream, addr)) => {
                    if let Ok(ucred) = stream.peer_cred() {
                        if let Some(addr) = addr.as_pathname() {
                            debug!(
                                "new ctl client from {} uid {} pid {}",
                                addr.display(),
                                ucred.uid(),
                                ucred.gid(),
                            );
                        } else {
                            debug!(
                                "new ctl client from uid {} pid {}",
                                ucred.uid(),
                                ucred.gid()
                            );
                        }
                    } else {
                        debug!("new ctl local control client");
                    }

                    let (r, w) = stream.into_split();
                    ctl_handle(r, w);
                }
                Err(e) => {
                    warn!("controller {} accept: {e}", self.listen_path.display());
                    break;
                }
            }
        }
    }
}

impl LocalController {
    fn start(self, mutex: &Mutex<Option<AbortHandle>>) -> anyhow::Result<impl Future> {
        let mut abort_handler_container = mutex.lock().unwrap();
        if abort_handler_container.is_some() {
            return Err(anyhow!("controller already existed"));
        }

        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let future = Abortable::new(self.into_running(), abort_registration);
        *abort_handler_container = Some(abort_handle);

        Ok(future)
    }

    fn abort(mutex: &Mutex<Option<AbortHandle>>) {
        let mut abort_handler_container = mutex.lock().unwrap();
        if let Some(abort_handle) = abort_handler_container.take() {
            abort_handle.abort();
        }
    }

    pub fn start_as_unique(self) -> anyhow::Result<impl Future> {
        let fut = self.start(&UNIQUE_CONTROLLER_ABORT_HANDLER)?;
        debug!("unique controller started");
        Ok(fut)
    }

    pub fn start_unique(daemon_group: &str) -> anyhow::Result<impl Future> {
        LocalController::create_unique(daemon_group)?.start_as_unique()
    }

    pub fn abort_unique() {
        LocalController::abort(&UNIQUE_CONTROLLER_ABORT_HANDLER);
    }

    pub fn start_as_daemon(self) -> anyhow::Result<impl Future> {
        let fut = self.start(&DAEMON_CONTROLLER_ABORT_HANDLER)?;
        debug!("daemon controller started");
        Ok(fut)
    }

    pub fn start_daemon(daemon_group: &str) -> anyhow::Result<impl Future> {
        LocalController::create_daemon(daemon_group)?.start_as_daemon()
    }

    pub fn abort_daemon() {
        LocalController::abort(&DAEMON_CONTROLLER_ABORT_HANDLER);
    }
}

#[cfg(unix)]
impl Drop for LocalController {
    fn drop(&mut self) {
        if self.listen_path.exists() {
            debug!("unlink socket file {}", self.listen_path.display());
            let _ = std::fs::remove_file(&self.listen_path);
        }
    }
}

#[cfg(unix)]
fn check_then_finalize_path(path: &Path) -> anyhow::Result<()> {
    if path.exists() {
        return Err(anyhow!(
            "control socket path {} already exists",
            path.display()
        ));
    }
    if !path.has_root() {
        return Err(anyhow!(
            "control socket path {} is not absolute",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        debug!("creating control directory {}", parent.display());
        DirBuilder::new().recursive(true).create(parent)?;
    }

    Ok(())
}
