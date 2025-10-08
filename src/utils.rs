use serde::de::StdError;

pub type Error = Box<dyn StdError + Send + Sync>;
