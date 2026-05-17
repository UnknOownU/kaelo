use rusqlite_migration::{Migrations, M};

pub fn migrations() -> Migrations<'static> {
    vec![
        M::up(include_str!("V001__initial.sql")),
        M::up(include_str!("V002__add_etag_columns.sql")),
        M::up(include_str!("V003__add_strategy_to_cache.sql")),
        M::up(include_str!("V004__diagnostics_and_evidence.sql")),
    ]
    .into_iter()
    .collect()
}
