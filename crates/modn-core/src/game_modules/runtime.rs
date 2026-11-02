use super::capabilities::{self, AllowedRoots, ReadProfile};
use super::loader::HOST_API_VERSION;
use anyhow::Result;
use mlua::{HookTriggers, Lua, LuaOptions, StdLib, Table, Value, VmState};
use std::cell::Cell;
use std::time::{Duration, Instant};

const MAX_VM_BYTES: usize = 8 * 1024 * 1024;
const MAX_HOOKS: u64 = 100_000;
const MAX_EXECUTION: Duration = Duration::from_secs(1);

pub(super) fn evaluate<T>(
    source: &str,
    label: &str,
    roots: AllowedRoots,
    profile: ReadProfile,
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
            "load",
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
        capabilities::register(&lua, &host, roots, profile)?;
        let proxy = lua.create_table()?;
        let meta = lua.create_table()?;
        meta.set("__index", host)?;
        meta.set(
            "__newindex",
            lua.create_function(|_, _: (Value, Value, Value)| {
                Err::<(), _>(mlua::Error::RuntimeError("mods_host is read-only".into()))
            })?,
        )?;
        meta.set("__metatable", false)?;
        proxy.set_metatable(Some(meta))?;
        globals.set("mods_host", proxy)?;
        let started = Instant::now();
        let hooks = Cell::new(0u64);
        lua.set_hook(
            HookTriggers::new().every_nth_instruction(1_000),
            move |_, _| {
                let next = hooks.get().saturating_add(1);
                hooks.set(next);
                if next > MAX_HOOKS || started.elapsed() > MAX_EXECUTION {
                    return Err(mlua::Error::RuntimeError(
                        "Lua module execution budget exceeded".into(),
                    ));
                }
                Ok(VmState::Continue)
            },
        )?;
        let output: Table = lua.load(source).set_name(label).eval()?;
        evaluate(&lua, output)
    };
    run().map_err(|error| anyhow::anyhow!("Lua module execution failed: {error}"))
}
