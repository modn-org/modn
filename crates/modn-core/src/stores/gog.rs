use super::{valid_store_id, GogRecord, StoreInstallation, Storefront};

pub(super) fn installations(records: &[GogRecord]) -> Vec<StoreInstallation> {
    records
        .iter()
        .filter(|record| {
            valid_store_id(Storefront::Gog, &record.product_id) && record.path.is_absolute()
        })
        .map(|record| StoreInstallation {
            store: Storefront::Gog,
            store_id: record.product_id.clone(),
            path: record.path.clone(),
        })
        .collect()
}
