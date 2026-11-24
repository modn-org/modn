-- Fixture policy for the executable-approval journey.
--
-- This game declares no content root, so the install plan writes below the
-- game folder. It recognizes a marker, a Lua file and a game-relative
-- `tools/*.exe`; the core still blocks the executable until the user approves
-- it.

local policy = {
    game_id = "tools-game",
    api_version = mods_host.api_version,
    marker = "Game.exe",
}

local function present(root_id, relative, kind)
    local ok, item = pcall(mods_host.fs.metadata, root_id, relative)
    return ok and item ~= nil and item.kind == kind
end

function policy.validate()
    return present(1, policy.marker, "file")
end

local function split_path(value)
    local parts = {}
    for part in string.gmatch(value, "[^/]+") do table.insert(parts, part) end
    return parts
end

local function is_tool_executable(relative)
    if string.sub(relative, -4) ~= ".exe" then return false end
    for _, part in ipairs(split_path(relative)) do
        if part == "tools" then return true end
    end
    return false
end

local function is_lua(relative)
    return string.sub(string.lower(relative), -4) == ".lua"
end

function policy.plan(files)
    local operations = {}
    local blockers = {}
    if #files == 0 then
        table.insert(blockers, { code = "EMPTY_PACKAGE", message = "Archive contains no files" })
        return { schema_version = 1, operations = operations, blockers = blockers }
    end
    for _, file in ipairs(files) do
        if is_lua(file.source) or is_tool_executable(file.source) then
            table.insert(operations, {
                source = file.source,
                destination = file.source,
                kind = "fixture-file",
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

function policy.scan(limit)
    local mods = {}
    local ok, listed = pcall(mods_host.directory.list, 2, "", math.max(limit, 1))
    if ok then
        for _, entry in ipairs(listed.items) do
            if #mods >= limit then break end
            if entry.kind == "file" and is_lua(entry.name) then
                table.insert(mods, { relative_path = entry.name, kind = "single", root = "game" })
            end
        end
    end
    return {
        mods = mods,
        framework_signals = {},
        incomplete = #mods >= limit,
        scan_issues = {},
        redmod_available = false,
    }
end

function policy.doctor(_report)
    return {}
end

return policy
