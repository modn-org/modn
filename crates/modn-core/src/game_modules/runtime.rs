//! Shared bounded Luau VM for external and bundled game policies.
use super::capabilities::{self, AllowedRoots};
use super::loader::HOST_API_VERSION;
use anyhow::Result;
use mlua::{Lua, LuaOptions, StdLib, Table, VmState};
use std::cell::Cell;
use std::time::{Duration, Instant};

const MAX_VM_BYTES: usize = 8 * 1024 * 1024;
const MAX_INTERRUPTS: u64 = 100_000;
const MAX_EXECUTION: Duration = Duration::from_secs(1);

pub(super) fn evaluate<T>(
    source: &str,
    label: &str,
    roots: AllowedRoots,
    evaluate: impl FnOnce(&Lua, Table) -> mlua::Result<T>,
) -> Result<T> {
    let run = || -> mlua::Result<T> {
        let lua = Lua::new_with(
            StdLib::TABLE | StdLib::STRING | StdLib::MATH,
            LuaOptions::default(),
        )?;
        lua.set_memory_limit(MAX_VM_BYTES)?;
        let globals = lua.globals();
        for restricted in [
            "io",
            "os",
            "package",
            "debug",
            "require",
            "loadstring",
            "dofile",
            "loadfile",
            "getfenv",
            "setfenv",
            "print",
            "warn",
        ] {
            globals.set(restricted, mlua::Value::Nil)?;
        }
        let host = lua.create_table()?;
        host.set("api_version", HOST_API_VERSION)?;
        capabilities::register(&lua, &host, roots)?;
        host.set_readonly(true);
        globals.set("mods_host", host)?;
        lua.sandbox(true)?;
        let started = Instant::now();
        let interrupts = Cell::new(0u64);
        lua.set_interrupt(move |_| {
            let next = interrupts.get().saturating_add(1);
            interrupts.set(next);
            if next > MAX_INTERRUPTS || started.elapsed() > MAX_EXECUTION {
                return Err(mlua::Error::RuntimeError(
                    "Luau module execution budget exceeded".into(),
                ));
            }
            Ok(VmState::Continue)
        });
        let output: Table = lua.load(source).set_name(label).eval()?;
        evaluate(&lua, output)
    };
    run().map_err(|error| anyhow::anyhow!("Luau module execution failed: {error}"))
}
