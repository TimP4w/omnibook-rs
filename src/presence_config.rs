use std::collections::HashMap;
use std::fs;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct PresenceConfig {
    // Proximity – when moving away
    pub prox_away_threshold: String, // "Very Near" | "Near" | "Far" | "Away"
    pub prox_away_delay: u32,        // seconds
    pub prox_away_action: String,    // "none" | "lock" | "custom"
    pub prox_away_custom: String,

    // Proximity – when returning
    pub prox_return_action: String,  // "none" | "wake" | "custom"
    pub prox_return_custom: String,

    // Attention – when not looking
    pub attn_notlooking_delay: u32,
    pub attn_notlooking_action: String, // "none" | "dim" | "lock" | "custom"
    pub attn_notlooking_dim: u8,        // 0–100 %
    pub attn_notlooking_custom: String,

    // Attention – when looking again
    pub attn_looking_action: String,    // "none" | "brighten" | "custom"
    pub attn_looking_bright: u8,        // 0–100 %
    pub attn_looking_custom: String,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            prox_away_threshold: "Far".to_string(),
            prox_away_delay: 30,
            prox_away_action: "none".to_string(),
            prox_away_custom: String::new(),
            prox_return_action: "wake".to_string(),
            prox_return_custom: String::new(),
            attn_notlooking_delay: 10,
            attn_notlooking_action: "none".to_string(),
            attn_notlooking_dim: 20,
            attn_notlooking_custom: String::new(),
            attn_looking_action: "none".to_string(),
            attn_looking_bright: 100,
            attn_looking_custom: String::new(),
        }
    }
}

impl PresenceConfig {
    pub fn load(path: &Path) -> Self {
        let content = fs::read_to_string(path).unwrap_or_default();
        Self::parse(&content)
    }

    pub fn parse(content: &str) -> Self {
        let mut cfg = Self::default();
        let map: HashMap<&str, &str> = content
            .lines()
            .filter_map(|l| {
                let mut it = l.splitn(2, '=');
                Some((it.next()?.trim(), it.next()?.trim()))
            })
            .collect();

        macro_rules! s {
            ($f:ident, $k:expr) => {
                if let Some(&v) = map.get($k) { cfg.$f = v.to_string(); }
            };
        }
        macro_rules! p {
            ($f:ident, $k:expr, $d:expr) => {
                if let Some(&v) = map.get($k) { cfg.$f = v.parse().unwrap_or($d); }
            };
        }

        s!(prox_away_threshold,    "prox_away_threshold");
        p!(prox_away_delay,        "prox_away_delay", 30);
        s!(prox_away_action,       "prox_away_action");
        s!(prox_away_custom,       "prox_away_custom");
        s!(prox_return_action,     "prox_return_action");
        s!(prox_return_custom,     "prox_return_custom");
        p!(attn_notlooking_delay,  "attn_notlooking_delay", 10);
        s!(attn_notlooking_action, "attn_notlooking_action");
        p!(attn_notlooking_dim,    "attn_notlooking_dim", 20);
        s!(attn_notlooking_custom, "attn_notlooking_custom");
        s!(attn_looking_action,    "attn_looking_action");
        p!(attn_looking_bright,    "attn_looking_bright", 100);
        s!(attn_looking_custom,    "attn_looking_custom");

        cfg
    }

    pub fn serialize(&self) -> String {
        format!(
            concat!(
                "prox_away_threshold={}\n",
                "prox_away_delay={}\n",
                "prox_away_action={}\n",
                "prox_away_custom={}\n",
                "prox_return_action={}\n",
                "prox_return_custom={}\n",
                "attn_notlooking_delay={}\n",
                "attn_notlooking_action={}\n",
                "attn_notlooking_dim={}\n",
                "attn_notlooking_custom={}\n",
                "attn_looking_action={}\n",
                "attn_looking_bright={}\n",
                "attn_looking_custom={}\n",
            ),
            self.prox_away_threshold, self.prox_away_delay,
            self.prox_away_action, self.prox_away_custom,
            self.prox_return_action, self.prox_return_custom,
            self.attn_notlooking_delay, self.attn_notlooking_action,
            self.attn_notlooking_dim, self.attn_notlooking_custom,
            self.attn_looking_action, self.attn_looking_bright,
            self.attn_looking_custom,
        )
    }

    pub fn save(&self, path: &Path) {
        let _ = fs::write(path, self.serialize());
    }

    /// Atomic write: write to a temp file then rename, preventing partial-write races.
    pub fn save_atomic(&self, path: &Path) {
        let tmp = path.with_extension("tmp");
        if fs::write(&tmp, self.serialize()).is_ok() {
            let _ = fs::rename(&tmp, path);
        }
    }

    /// Returns true if `state` meets or exceeds the configured away threshold.
    /// Levels: Very Near (0) < Near (1) < Far (2) < Away (3)
    pub fn prox_meets_away_threshold(&self, state: &str) -> bool {
        state_level(state) >= state_level(&self.prox_away_threshold)
    }
}

pub fn state_level(s: &str) -> u8 {
    match s {
        "Very Near" => 0,
        "Near"      => 1,
        "Far"       => 2,
        "Away"      => 3,
        _           => 3,
    }
}
