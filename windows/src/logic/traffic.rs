use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrafficSample {
    #[serde(default)]
    pub up: u64,
    #[serde(default)]
    pub down: u64,
    #[serde(default, rename = "upTotal")]
    pub up_total: Option<u64>,
    #[serde(default, rename = "downTotal")]
    pub down_total: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TrafficLedger {
    #[serde(default)]
    pub lifetime_upload: u64,
    #[serde(default)]
    pub lifetime_download: u64,
    #[serde(default)]
    pub last_upload: u64,
    #[serde(default)]
    pub last_download: u64,
    #[serde(default)]
    pub run_id: Option<String>,
}

impl TrafficLedger {
    pub fn record(&mut self, upload: u64, download: u64, run_id: &str) {
        let same_run = self.run_id.as_deref() == Some(run_id);
        let upload_delta = if same_run && upload >= self.last_upload {
            upload - self.last_upload
        } else {
            upload
        };
        let download_delta = if same_run && download >= self.last_download {
            download - self.last_download
        } else {
            download
        };
        self.lifetime_upload = self.lifetime_upload.saturating_add(upload_delta);
        self.lifetime_download = self.lifetime_download.saturating_add(download_delta);
        self.last_upload = upload;
        self.last_download = download;
        self.run_id = Some(run_id.to_string());
    }
}

pub fn format_bytes(value: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}
