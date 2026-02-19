use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub type TxHash = [u8; 32];
pub type BlockHash = [u8; 32];
pub type Address = [u8; 20];
pub type PeerId = String;

#[derive(Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct SourceId(pub String);

impl SourceId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl Display for SourceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::SourceId;

    #[test]
    fn source_id_display_matches_inner_value() {
        let source = SourceId::new("peer-1");
        assert_eq!(source.to_string(), "peer-1");
    }
}
