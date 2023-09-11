#!/bin/sh

set -e

SCRIPT_DIR=$(dirname $0)
TMP_FILE=$(mktemp /tmp/countryInfo.txt.XXXXXX)

curl "https://download.geonames.org/export/dump/countryInfo.txt" -o ${TMP_FILE}

cd "${SCRIPT_DIR}"

cat << EOF > ../../../lib/g3-geoip/src/country/iso_generated.rs
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

///
/// This file is auto generated by scripts/generate/country/convert.sh,
/// by using the data from https://download.geonames.org/export/dump/countryInfo.txt
///
use std::fmt;
use std::str::FromStr;

use crate::ContinentCode;

$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_names.awk)

$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_alpha2.awk)

$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_alpha3.awk)

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum IsoCountryCode {
$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_enum.awk)
}

impl IsoCountryCode {
    pub fn name(&self) -> &'static str {
        ALL_COUNTRY_NAMES[*self as usize]
    }

    pub fn alpha2_code(&self) -> &'static str {
        ALL_ALPHA2_CODES[*self as usize]
    }

    pub fn alpha3_code(&self) -> &'static str {
        ALL_ALPHA3_CODES[*self as usize]
    }

$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_variant_count.awk)

$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_continent.awk)
}

impl fmt::Display for IsoCountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.alpha2_code(), self.name())
    }
}

impl FromStr for IsoCountryCode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.len() {
            2 => match s {
$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_from_alpha2_str.awk)
                _ => Err(()),
            },
            3 => match s {
$(cat ${TMP_FILE} | awk -F'\t' -f iso3166_from_alpha3_str.awk)
                _ => Err(()),
            },
            _ => Err(()),
        }
    }
}
EOF

rm ${TMP_FILE}

