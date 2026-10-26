//! A minimal Sven Co-op plugin on the safe metamod-p API: it logs when it
//! attaches, when a client connects, when a map starts, the first model the
//! game precaches and when the game shuts down, and adds the `rs_hello`
//! command and the `rs_greeting` cvar.

use std::{cell::Cell, error::Error, ffi::CStr};

use metamod_p::{
    CvarFlags, Edict, Engine, LoadTime, MetaResult, Plugin, PluginInfo, PostResult, RejectReason,
    Returned, StaticCvar,
};

static GREETING: StaticCvar = StaticCvar::new(
    c"rs_greeting",
    c"Hello from Rust",
    CvarFlags::SERVER.union(CvarFlags::EXTDLL),
);

struct Hello {
    engine:          &'static Engine,
    /// A map precaches hundreds of models; only the first one is logged.
    logged_precache: Cell<bool>,
}

#[metamod_p::plugin]
impl Plugin for Hello {
    const INFO: PluginInfo = PluginInfo {
        name:       c"Rust Hello",
        version:    c"0.1.0",
        date:       c"2026-10-27",
        author:     c"Steve Fan",
        url:        c"https://github.com/stevefan1999-personal/metamod-p-rs",
        log_tag:    c"RSHELLO",
        loadable:   LoadTime::AnyTime,
        unloadable: LoadTime::AnyPause,
    };

    fn attach(engine: &'static Engine, now: LoadTime) -> Result<Self, Box<dyn Error>> {
        let greeting = GREETING.register(engine)?;
        engine.register_command(c"rs_hello", move |args| {
            log::info!(
                "rs_hello: {} (arguments: {:?})",
                greeting.string(),
                args.rest()
            );
        });
        log::info!("attached at {now:?}");
        Ok(Self {
            engine,
            logged_precache: Cell::default(),
        })
    }

    fn client_connect(
        &self,
        player: Edict,
        name: &CStr,
        address: &CStr,
        _reject_reason: &mut RejectReason,
    ) -> MetaResult<bool> {
        log::info!(
            "{name:?} connects from {address:?} into slot {}",
            player.index()
        );
        MetaResult::Ignored
    }

    fn server_activate(&self, edict_count: i32, max_clients: i32) -> MetaResult<()> {
        let map = self.engine.map_name().unwrap_or_default();
        log::info!("map {map} is active with {edict_count} edicts and {max_clients} player slots");
        MetaResult::Ignored
    }

    // A post hook on an engine function: it reads what the engine returned.
    fn precache_model_post(&self, path: &CStr, returned: Returned<i32>) -> PostResult<i32> {
        if !self.logged_precache.replace(true) {
            log::info!("first precache: {path:?} got {returned:?}");
        }
        PostResult::Ignored
    }

    // A hook on the extra game DLL functions (NEW_DLL_FUNCTIONS).
    fn game_shutdown(&self) -> MetaResult<()> {
        log::info!("the game shuts down");
        MetaResult::Ignored
    }
}
