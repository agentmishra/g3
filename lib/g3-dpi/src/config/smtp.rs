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

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmtpInterceptionConfig {
    pub greeting_timeout: Duration,
    pub quit_wait_timeout: Duration,
}

impl Default for SmtpInterceptionConfig {
    fn default() -> Self {
        SmtpInterceptionConfig {
            greeting_timeout: Duration::from_secs(300),
            quit_wait_timeout: Duration::from_secs(60),
        }
    }
}
