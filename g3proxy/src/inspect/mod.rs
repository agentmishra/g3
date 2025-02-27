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

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use slog::Logger;
use tokio::io::{AsyncRead, AsyncWrite};
use uuid::Uuid;

use g3_daemon::server::ServerQuitPolicy;
use g3_dpi::{
    H1InterceptionConfig, H2InterceptionConfig, MaybeProtocol, ProtocolInspectPolicy,
    ProtocolInspector, SmtpInterceptionConfig,
};
use g3_types::net::OpensslClientConfig;

use crate::audit::AuditHandle;
use crate::auth::{User, UserForbiddenStats, UserSite};
use crate::config::server::ServerConfig;
use crate::serve::{ArcServerStats, ServerIdleChecker, ServerTaskNotes};

mod error;
pub(crate) use error::InterceptionError;

pub(crate) mod stream;

pub(crate) mod tls;
use tls::TlsInterceptionContext;

pub(crate) mod http;
mod websocket;

pub(crate) mod smtp;

#[derive(Clone)]
pub(super) struct StreamInspectUserContext {
    raw_user_name: Option<String>,
    user: Arc<User>,
    user_site: Option<Arc<UserSite>>,
    forbidden_stats: Arc<UserForbiddenStats>,
}

impl StreamInspectUserContext {
    fn http_rsp_hdr_recv_timeout(&self) -> Option<Duration> {
        self.user_site
            .as_ref()
            .and_then(|site| site.http_rsp_hdr_recv_timeout())
            .or(self.user.http_rsp_hdr_recv_timeout())
    }
}

#[derive(Clone)]
pub(super) struct StreamInspectTaskNotes {
    task_id: Uuid,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
    worker_id: Option<usize>,
    user_ctx: Option<StreamInspectUserContext>,
}

impl From<&ServerTaskNotes> for StreamInspectTaskNotes {
    fn from(task_notes: &ServerTaskNotes) -> Self {
        StreamInspectTaskNotes {
            task_id: task_notes.id,
            client_addr: task_notes.client_addr(),
            server_addr: task_notes.server_addr(),
            worker_id: task_notes.worker_id(),
            user_ctx: task_notes.user_ctx().map(|ctx| StreamInspectUserContext {
                raw_user_name: ctx.raw_user_name().map(|s| s.to_string()),
                user: ctx.user().clone(),
                user_site: ctx.user_site().cloned(),
                forbidden_stats: ctx.forbidden_stats().clone(),
            }),
        }
    }
}

pub(crate) struct StreamInspectContext<SC: ServerConfig> {
    audit_handle: Arc<AuditHandle>,
    server_config: Arc<SC>,
    server_stats: ArcServerStats,
    server_quit_policy: Arc<ServerQuitPolicy>,
    task_notes: StreamInspectTaskNotes,
    inspection_depth: usize,

    task_max_idle_count: i32,
}

impl<SC: ServerConfig> Clone for StreamInspectContext<SC> {
    fn clone(&self) -> Self {
        StreamInspectContext {
            audit_handle: self.audit_handle.clone(),
            server_config: self.server_config.clone(),
            server_stats: self.server_stats.clone(),
            server_quit_policy: self.server_quit_policy.clone(),
            task_notes: self.task_notes.clone(),
            inspection_depth: self.inspection_depth,
            task_max_idle_count: self.task_max_idle_count,
        }
    }
}

impl<SC: ServerConfig> StreamInspectContext<SC> {
    pub(crate) fn new(
        audit_handle: Arc<AuditHandle>,
        server_config: Arc<SC>,
        server_stats: ArcServerStats,
        server_quit_policy: Arc<ServerQuitPolicy>,
        task_notes: &ServerTaskNotes,
    ) -> Self {
        let mut task_max_idle_count = server_config.task_max_idle_count();
        if let Some(user_ctx) = task_notes.user_ctx() {
            task_max_idle_count = user_ctx.user().task_max_idle_count();
        }

        StreamInspectContext {
            audit_handle,
            server_config,
            server_stats,
            server_quit_policy,
            task_notes: StreamInspectTaskNotes::from(task_notes),
            inspection_depth: 0,
            task_max_idle_count,
        }
    }

    fn user(&self) -> Option<&Arc<User>> {
        self.task_notes.user_ctx.as_ref().map(|cx| &cx.user)
    }

    fn raw_user_name(&self) -> Option<&str> {
        self.task_notes
            .user_ctx
            .as_ref()
            .and_then(|cx| cx.raw_user_name.as_deref())
    }

    #[inline]
    pub(crate) fn server_task_id(&self) -> &Uuid {
        &self.task_notes.task_id
    }

    #[inline]
    fn server_force_quit(&self) -> bool {
        self.server_quit_policy.force_quit()
    }

    #[inline]
    fn server_offline(&self) -> bool {
        !self.server_stats.is_online()
    }

    #[inline]
    pub(crate) fn inspect_logger(&self) -> &Logger {
        self.audit_handle.inspect_logger()
    }

    #[inline]
    pub(crate) fn intercept_logger(&self) -> &Logger {
        self.audit_handle.intercept_logger()
    }

    pub(crate) fn idle_checker(&self) -> ServerIdleChecker {
        ServerIdleChecker {
            idle_duration: self.server_config.task_idle_check_duration(),
            user: self.user().cloned(),
            task_max_idle_count: self.task_max_idle_count,
            server_quit_policy: self.server_quit_policy.clone(),
        }
    }

    pub(crate) fn protocol_inspector(
        &self,
        explicit_protocol: Option<MaybeProtocol>,
    ) -> ProtocolInspector {
        let mut inspector = ProtocolInspector::new(
            self.audit_handle.server_tcp_portmap(),
            self.audit_handle.client_tcp_portmap(),
        );
        if let Some(p) = explicit_protocol {
            inspector.push_protocol(p);
        }
        inspector
    }

    #[inline]
    pub(crate) fn current_inspection_depth(&self) -> usize {
        self.inspection_depth
    }

    #[inline]
    fn increase_inspection_depth(&mut self) {
        self.inspection_depth += 1;
    }

    #[inline]
    pub(crate) fn tls_interception(&self) -> Option<TlsInterceptionContext> {
        self.audit_handle.tls_interception()
    }

    pub(crate) fn user_site_tls_client(&self) -> Option<&OpensslClientConfig> {
        self.task_notes
            .user_ctx
            .as_ref()
            .and_then(|v| v.user_site.as_ref())
            .and_then(|v| v.tls_client())
    }

    fn log_uri_max_chars(&self) -> usize {
        self.task_notes
            .user_ctx
            .as_ref()
            .and_then(|cx| cx.user.log_uri_max_chars())
            .unwrap_or_else(|| self.audit_handle.log_uri_max_chars())
    }

    #[inline]
    fn h1_interception(&self) -> &H1InterceptionConfig {
        self.audit_handle.h1_interception()
    }

    fn h1_rsp_hdr_recv_timeout(&self) -> Duration {
        self.task_notes
            .user_ctx
            .as_ref()
            .and_then(|ctx| ctx.http_rsp_hdr_recv_timeout())
            .unwrap_or(self.h1_interception().rsp_head_recv_timeout)
    }

    #[inline]
    fn h2_inspect_policy(&self) -> ProtocolInspectPolicy {
        self.audit_handle.h2_inspect_policy()
    }

    #[inline]
    fn h2_interception(&self) -> &H2InterceptionConfig {
        self.audit_handle.h2_interception()
    }

    fn h2_rsp_hdr_recv_timeout(&self) -> Duration {
        self.task_notes
            .user_ctx
            .as_ref()
            .and_then(|ctx| ctx.http_rsp_hdr_recv_timeout())
            .unwrap_or(self.h2_interception().rsp_head_recv_timeout)
    }

    #[inline]
    fn smtp_inspect_policy(&self) -> ProtocolInspectPolicy {
        self.audit_handle.smtp_inspect_policy()
    }

    #[inline]
    fn smtp_interception(&self) -> &SmtpInterceptionConfig {
        self.audit_handle.smtp_interception()
    }

    #[inline]
    fn task_max_idle_count(&self) -> i32 {
        self.task_max_idle_count
    }

    fn belongs_to_blocked_user(&self) -> bool {
        self.task_notes
            .user_ctx
            .as_ref()
            .map(|cx| cx.user.is_blocked())
            .unwrap_or(false)
    }
}

pub(crate) enum StreamInspection<SC: ServerConfig> {
    End,
    StreamUnknown(stream::StreamInspectObject<SC>),
    StreamInspect(stream::StreamInspectObject<SC>),
    TlsModern(tls::TlsInterceptObject<SC>),
    #[cfg(feature = "vendored-tongsuo")]
    TlsTlcp(tls::TlsInterceptObject<SC>),
    H1(http::H1InterceptObject<SC>),
    H2(http::H2InterceptObject<SC>),
    Websocket(websocket::H1WebsocketInterceptObject<SC>),
    Smtp(smtp::SmtpInterceptObject<SC>),
}

type BoxAsyncRead = Box<dyn AsyncRead + Send + Unpin + 'static>;
type BoxAsyncWrite = Box<dyn AsyncWrite + Send + Unpin + 'static>;
