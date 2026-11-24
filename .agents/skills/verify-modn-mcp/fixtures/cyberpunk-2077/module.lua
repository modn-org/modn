-- Minimal fixture policy for the MCP verifier.
--
-- It recognizes the Cyberpunk 2077 marker and plans every `.archive` file
-- below `archive/pc/mod/` to the same game-relative destination. It is not a
-- compatibility claim for the real game.

local policy = {
    game_id = "cyberpunk-2077",
    api_version = mods_host.api_version,
    marker = "bin/x64/Cyberpunk2077.exe",
}

local function is_archive(relative)
    return string.sub(relative, 1, 15) == "archive/pc/mod/"
        and string.sub(relative, -8) == ".archive"
end

function policy.validate()
    local ok, item = pcall(mods_host.fs.metadata, 1, policy.marker)
    return ok and item ~= nil and item.kind == "file"
end

function policy.plan(files)
    local operations = {}
    local blockers = {}
    if #files == 0 then
        table.insert(blockers, { code = "EMPTY_PACKAGE", message = "Archive contains no files" })
    end
    for _, file in ipairs(files) do
        if is_archive(file.source) then
            table.insert(operations, {
                source = file.source,
                destination = file.source,
                kind = "archive",
                package_root = "",
            })
        else
            table.insert(blockers, {
                code = "UNRECOGNIZED_LAYOUT",
                message = "Unsupported fixture file: " .. file.source,
            })
        end
    end
    return { schema_version = 1, operations = operations, blockers = blockers }
end

function policy.scan(_limit)
    return {
        mods = {},
        framework_signals = {},
        incomplete = false,
        scan_issues = {},
        redmod_available = false,
    }
end

function policy.doctor(_report)
    return {}
end

return policy
