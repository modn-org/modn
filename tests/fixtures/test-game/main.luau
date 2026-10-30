-- Fictional game policy. Uses only public, read-only host capabilities.
-- Deliberately unrelated to any real game's installation layout.
local fs = mods_host.fs
local directory = mods_host.directory

local policy = {
    game_id = "test-game",
    api_version = mods_host.api_version,
    marker = "bin/TestGame.exe",
    steam_manifest = "appmanifest_000000.acf",
    common_paths = {},
}

local function metadata(path)
    local ok, info = pcall(fs.metadata, 1, path)
    if ok then return info end
    return nil
end

function policy.validate()
    local marker = metadata(policy.marker)
    return marker ~= nil and marker.kind == "file"
end

function policy.steam_name()
    return "Test Game"
end

function policy.is_epic(name)
    return name == "Test Game"
end

function policy.scan(limit)
    local found = {}
    local incomplete = false
    local stack = { "addons" }
    local visited = 0

    while #stack > 0 do
        local folder = table.remove(stack)
        local ok, result = pcall(directory.list, 1, folder)
        if ok then
            if result.incomplete then incomplete = true end
            for _, entry in ipairs(result.items) do
                visited += 1
                if visited > limit then
                    incomplete = true
                    break
                end
                local relative = folder .. "/" .. entry.name
                if entry.kind == "directory" then
                    table.insert(stack, relative)
                elseif entry.kind == "file" and string.sub(string.lower(entry.name), -7) == ".plugin" then
                    table.insert(found, { relative_path = relative, kind = "test-plugin" })
                end
            end
        elseif folder ~= "addons" then
            incomplete = true
        end
        if incomplete then break end
    end

    table.sort(found, function(a, b) return a.relative_path < b.relative_path end)
    return {
        mods = found,
        framework_signals = {},
        incomplete = incomplete,
        redmod_available = false,
    }
end

function policy.doctor(report)
    local result = {}
    if report.incomplete then
        table.insert(result, {
            code = "SCAN_INCOMPLETE", severity = "warning",
            message = "Fixture scan reached an entry limit.",
        })
    end
    for _, mod in ipairs(report.mods) do
        if not mod.managed then
            table.insert(result, {
                code = "UNMANAGED_CONTENT", severity = "warning",
                message = "Fixture includes unmanaged add-ons.",
            })
            break
        end
    end
    for _, file in ipairs(report.owned_files) do
        if file.status ~= "intact" then
            table.insert(result, {
                code = "OWNED_FILE_DRIFT", severity = "warning",
                message = "A managed add-on is missing or modified.",
            })
            break
        end
    end
    return result
end

-- Source: [optional wrapper]/package/plugins/<name>.plugin
-- Destination: addons/<name>.plugin. No Rust-side game exceptions.
function policy.plan(files)
    local operations = {}
    local blockers = {}
    local function block(code, source)
        table.insert(blockers, { code = code, message = "Unmapped fixture entry: " .. source })
    end
    for _, file in ipairs(files) do
        local source = file.source
        local prefix = "package/plugins/"
        local start = string.find(source, prefix, 1, true)
        local before = start and string.sub(source, 1, start - 1) or ""
        local suffix = start and string.sub(source, start + #prefix) or ""
        local wrapper = string.sub(before, 1, -2)
        if start ~= nil and (start == 1 or string.sub(before, -1) == "/")
            and suffix ~= "" and string.sub(string.lower(suffix), -7) == ".plugin" then
            table.insert(operations, {
                source = source,
                destination = "addons/" .. suffix,
                kind = "test-plugin",
                package_root = wrapper,
            })
        elseif not (string.lower(source) == "readme.md"
            or string.lower(source) == "license.txt") then
            block("UNRECOGNIZED_LAYOUT", source)
        end
    end
    return { schema_version = 1, operations = operations, blockers = blockers }
end

-- Group a nested add-on as a package; top-level files are individual packages.
function policy.adoption(mods)
    local groups = {}
    local order = {}
    for _, item in ipairs(mods) do
        if item.kind == "test-plugin" then
            local selector = string.match(item.relative_path, "^(addons/[^/]+)/")
                or item.relative_path
            if groups[selector] == nil then
                groups[selector] = { selector = selector, kind = item.kind, files = {} }
                table.insert(order, selector)
            end
            table.insert(groups[selector].files, item.relative_path)
        end
    end
    table.sort(order)
    local output = {}
    for _, selector in ipairs(order) do
        table.sort(groups[selector].files)
        table.insert(output, groups[selector])
    end
    return { schema_version = 1, groups = output }
end

return policy
