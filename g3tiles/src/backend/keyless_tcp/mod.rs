/*
 * Copyright 2024 ByteDance and/or its affiliates.
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
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context};
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use futures_util::future::{AbortHandle, Abortable};

use g3_types::collection::{SelectiveVec, SelectiveVecBuilder, WeightedValue};
use g3_types::metrics::MetricsName;

use super::{ArcBackend, Backend};
use crate::config::backend::keyless_tcp::KeylessTcpBackendConfig;
use crate::config::backend::{AnyBackendConfig, BackendConfig};
use crate::module::keyless::{
    KeylessBackendStats, KeylessRequest, KeylessResponse, KeylessUpstreamDurationRecorder,
    KeylessUpstreamDurationStats,
};

pub(crate) struct KeylessTcpBackend {
    config: Arc<KeylessTcpBackendConfig>,
    site_stats: Arc<KeylessBackendStats>,
    duration_recorder: Arc<KeylessUpstreamDurationRecorder>,
    duration_stats: Arc<KeylessUpstreamDurationStats>,
    peer_addrs: Arc<ArcSwapOption<SelectiveVec<WeightedValue<SocketAddr>>>>,
    discover_handle: Mutex<Option<AbortHandle>>,
}

impl KeylessTcpBackend {
    fn new_obj(
        config: Arc<KeylessTcpBackendConfig>,
        site_stats: Arc<KeylessBackendStats>,
        duration_recorder: Arc<KeylessUpstreamDurationRecorder>,
        duration_stats: Arc<KeylessUpstreamDurationStats>,
    ) -> anyhow::Result<ArcBackend> {
        let peer_addrs = Arc::new(ArcSwapOption::new(None));

        // always update extra metrics tags
        site_stats.set_extra_tags(config.extra_metrics_tags.clone());
        duration_stats.set_extra_tags(config.extra_metrics_tags.clone());

        let backend = Arc::new(KeylessTcpBackend {
            config,
            site_stats,
            duration_recorder,
            duration_stats,
            peer_addrs,
            discover_handle: Mutex::new(None),
        });
        backend.update_discover()?;

        Ok(backend)
    }

    pub(super) fn prepare_initial(config: KeylessTcpBackendConfig) -> anyhow::Result<ArcBackend> {
        let site_stats = Arc::new(KeylessBackendStats::new(config.name()));
        let (duration_recorder, duration_stats) =
            KeylessUpstreamDurationRecorder::new(config.name(), &config.duration_stats);
        let duration_stats = Arc::new(duration_stats);

        crate::stat::metrics::backend::keyless::push_keyless_stats(site_stats.clone());
        crate::stat::metrics::backend::keyless::push_keyless_duration_stats(duration_stats.clone());

        KeylessTcpBackend::new_obj(
            Arc::new(config),
            site_stats,
            Arc::new(duration_recorder),
            duration_stats,
        )
    }

    fn prepare_reload(&self, config: KeylessTcpBackendConfig) -> anyhow::Result<ArcBackend> {
        let stats = self.site_stats.clone();
        // TODO reuse old connection pool?
        KeylessTcpBackend::new_obj(
            Arc::new(config),
            stats,
            self.duration_recorder.clone(),
            self.duration_stats.clone(),
        )
    }
}

#[async_trait]
impl Backend for KeylessTcpBackend {
    fn _clone_config(&self) -> AnyBackendConfig {
        AnyBackendConfig::KeylessTcp(self.config.as_ref().clone())
    }

    fn _update_config_in_place(
        &self,
        _flags: u64,
        _config: AnyBackendConfig,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn _lock_safe_reload(&self, config: AnyBackendConfig) -> anyhow::Result<ArcBackend> {
        if let AnyBackendConfig::KeylessTcp(c) = config {
            self.prepare_reload(c)
        } else {
            Err(anyhow!("invalid backend config type"))
        }
    }

    #[inline]
    fn name(&self) -> &MetricsName {
        self.config.name()
    }

    fn discover(&self) -> &MetricsName {
        &self.config.discover
    }
    fn update_discover(&self) -> anyhow::Result<()> {
        let discover = &self.config.discover;
        let discover = crate::discover::get_discover(discover)?;
        let mut discover_receiver = discover
            .register_data(&self.config.discover_data)
            .context("failed to register to discover {discover}")?;

        let peer_addrs_container = self.peer_addrs.clone();
        let (abort_handle, abort_reg) = AbortHandle::new_pair();
        let abort_fut = Abortable::new(
            async move {
                while discover_receiver.changed().await.is_ok() {
                    if let Ok(data) = discover_receiver.borrow().as_ref() {
                        let mut builder = SelectiveVecBuilder::new();
                        for v in data {
                            builder.insert(*v);
                        }
                        peer_addrs_container.store(builder.build().map(Arc::new));
                    }
                }
            },
            abort_reg,
        );

        let mut guard = self.discover_handle.lock().unwrap();
        if let Some(old_handle) = guard.replace(abort_handle) {
            old_handle.abort();
        }
        drop(guard);

        tokio::spawn(abort_fut);

        Ok(())
    }

    async fn keyless(&self, req: &KeylessRequest) -> KeylessResponse {
        todo!()
    }
}
