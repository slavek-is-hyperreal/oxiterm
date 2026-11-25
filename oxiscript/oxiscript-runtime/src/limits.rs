//! Resource limits. Everything a script can consume is bounded by one of
//! these numbers; the defaults suit a UI script on a desktop host.

#[derive(Clone, Debug)]
pub struct Limits {
    /// Largest accepted `.wasm` (bytes), checked before any parsing.
    pub max_module_bytes: usize,
    /// Fuel for every guest entry (`oxi_init`, a callback, an export, a tick,
    /// a release). Fuel is reset at each entry, never carried over.
    pub fuel_per_call: u64,
    /// Fuel charged for every host (import) call, on top of the guest
    /// instructions that led to it.
    pub host_call_fuel: u64,
    /// One unit of fuel per this many bytes copied across the boundary
    /// (arguments and results). Must be >= 1.
    pub bytes_per_fuel: u64,
    /// Linear memory ceiling in 64 KiB pages (initial size included).
    pub max_memory_pages: u32,
    /// Largest table (elements).
    pub max_table_elements: u32,
    /// Call depth (Wasm frames).
    pub max_recursion_depth: usize,
    /// Value stack height limit (engine cells).
    pub max_stack_height: usize,
    /// Live instances per [`crate::Host`], counting the transient second
    /// instance during a reload.
    pub max_instances: usize,
    /// Largest string/bytes the host copies in either direction in one call.
    pub max_host_payload: u32,
    /// Callbacks delivered per [`crate::Runtime::pump`] (a watcher that
    /// writes the state it watches would otherwise loop forever).
    pub max_events_per_pump: usize,
    /// Aggregate budget of one `pump`: it stops starting new guest entries
    /// once this much fuel was spent (the first entry always runs, so a pump
    /// always makes progress). Leftovers carry to the next pump; events and
    /// releases alternate, so neither starves the other.
    pub pump_fuel: u64,
    /// Wall-time cap of one pump in microseconds (0 = off). Needs a clock
    /// (`Host::set_clock`); makes pump non-deterministic, leave it unset in
    /// deterministic tests.
    pub pump_wall_micros: u64,
    /// Bound of the release queue (guest closures the host dropped).
    pub max_pending_releases: usize,
    /// Largest `oxi.manifest` section.
    pub max_manifest_bytes: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_module_bytes: 4 << 20,
            fuel_per_call: 10_000_000,
            host_call_fuel: 200,
            bytes_per_fuel: 16,
            max_memory_pages: 256, // 16 MiB
            max_table_elements: 10_000,
            max_recursion_depth: 512,
            max_stack_height: 256 * 1024,
            max_instances: 16,
            max_host_payload: 64 * 1024,
            max_events_per_pump: 256,
            pump_fuel: 30_000_000,
            pump_wall_micros: 50_000,
            max_pending_releases: 1024,
            max_manifest_bytes: 4096,
        }
    }
}
