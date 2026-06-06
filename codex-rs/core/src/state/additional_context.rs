use std::collections::BTreeMap;

use crate::context::AdditionalContextDeveloperFragment;
use crate::context::AdditionalContextUserFragment;
use crate::context::ContextualUserFragment;
use codex_protocol::models::ResponseInputItem;
use codex_protocol::protocol::AdditionalContextEntry;
use codex_protocol::protocol::AdditionalContextKind;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AdditionalContextStore {
    values: BTreeMap<String, AdditionalContextEntry>,
}

impl AdditionalContextStore {
    pub(crate) fn clear(&mut self) {
        self.values.clear();
    }

    pub(crate) fn merge(
        &mut self,
        values: BTreeMap<String, AdditionalContextEntry>,
    ) -> Vec<ResponseInputItem> {
        let fragments = values
            .iter()
            .filter(|(key, value)| self.values.get(*key) != Some(*value))
            .map(|(key, entry)| match entry.kind {
                AdditionalContextKind::Untrusted => {
                    AdditionalContextUserFragment::new(key.clone(), entry.value.clone())
                        .into_response_input_item()
                }
                AdditionalContextKind::Application => {
                    AdditionalContextDeveloperFragment::new(key.clone(), entry.value.clone())
                        .into_response_input_item()
                }
            })
            .collect();
        self.values = values;
        fragments
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context_values() -> BTreeMap<String, AdditionalContextEntry> {
        BTreeMap::from([(
            "codex_context_pin_pin-1".to_string(),
            AdditionalContextEntry {
                value: "Remember this".to_string(),
                kind: AdditionalContextKind::Untrusted,
            },
        )])
    }

    #[test]
    fn clear_allows_unchanged_context_to_be_reemitted() {
        let mut store = AdditionalContextStore::default();

        assert_eq!(store.merge(context_values()).len(), 1);
        assert_eq!(store.merge(context_values()).len(), 0);

        store.clear();

        assert_eq!(store.merge(context_values()).len(), 1);
    }
}
