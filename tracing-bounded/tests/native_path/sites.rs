//! Native-path inventory: these four macro sites are never warmed up.

pub fn first() {
    tracing::info!(target: "audit", parent: None, sequence = 0_u64);
}

pub fn second() {
    tracing::info!(target: "audit", parent: None, sequence = 1_u64);
}

pub fn filtered() {
    tracing::info!(target: "excluded", parent: None, sequence = 2_u64);
}

pub fn rejected(value: &dyn std::fmt::Debug) {
    tracing::info!(target: "audit", parent: None, value = ?value);
}
