-- Cyberpunk 2077 discovery, scan layout and diagnostics policy.
-- All I/O is mediated by Rust's read-only, root-scoped host services.
local fs = mods_host.fs
local dir = mods_host.directory

local policy = {
    game_id = "cyberpunk-2077",
    api_version = mods_host.api_version,
    marker = "bin/x64/Cyberpunk2077.exe",
    steam_manifest = "appmanifest_1091500.acf",
    common_paths = {
        "GOG Galaxy/Games/Cyberpunk 2077",
        "Epic Games/Cyberpunk2077",
        "Epic Games/Cyberpunk 2077",
    },
}

local function metadata(relative)
    local ok, value = pcall(fs.metadata, 1, relative)
    if ok then return value end
    return nil
end

local function present(relative, kind)
    local item = metadata(relative)
    return item ~= nil and item.kind == kind
end

function policy.validate()
    return present(policy.marker, "file")
end

local function quoted_value(text, key)
    for line in string.gmatch(text, "[^\r\n]+") do
        local words = {}
        for word in string.gmatch(line, '"([^"]*)"') do
            table.insert(words, word)
        end
        if #words >= 2 and string.lower(words[1]) == string.lower(key) then
            return words[2]
        end
    end
    return nil
end

function policy.steam_name()
    local ok, contents = pcall(dir.read_text, 1, "steamapps/appmanifest_1091500.acf")
    local name = ok and quoted_value(contents, "installdir") or nil
    if type(name) ~= "string" or name == "" or name == "." or name == ".."
        or string.find(name, "[/\\]") then
        return "Cyberpunk 2077"
    end
    return name
end

function policy.is_epic(display_name)
    return display_name == "Cyberpunk 2077"
end

local function listing(folder)
    local ok, response = pcall(dir.list, 1, folder)
    if ok then return response end
    return nil
end

function policy.scan(limit)
    local mods = {}
    local signals = {}
    local remaining = limit
    local incomplete = false

    local function push(relative, kind)
        table.insert(mods, { relative_path = relative, kind = kind })
    end

    local function consume(items)
        if remaining <= 0 then
            incomplete = true
            return false
        end
        remaining -= 1
        return true
    end

    local function collect_files(folder, extensions, kind)
        local stack = { folder }
        while #stack > 0 do
            local current = table.remove(stack)
            local response = listing(current)
            if response ~= nil then
                if response.incomplete then incomplete = true end
                for _, entry in ipairs(response.items) do
                    if not consume(entry) then return end
                    local relative = current .. "/" .. entry.name
                    if entry.kind == "directory" then
                        table.insert(stack, relative)
                    elseif entry.kind == "file" then
                        local ext = string.match(entry.name, "%.([^%.]+)$")
                        if ext ~= nil and extensions[string.lower(ext)] then
                            push(relative, kind)
                        end
                    end
                end
            end
        end
    end

    local function collect_directories(folder, kind)
        local response = listing(folder)
        if response == nil then return end
        if response.incomplete then incomplete = true end
        for _, entry in ipairs(response.items) do
            if not consume(entry) then return end
            if entry.kind == "directory" then
                push(folder .. "/" .. entry.name, kind)
            end
        end
    end

    collect_files("archive/pc/mod", { archive = true }, "legacy-archive")
    collect_files("r6/scripts", { reds = true }, "redscript")
    collect_files("r6/tweaks", { yaml = true, yml = true }, "tweak")
    collect_directories("mods", "redmod-directory")
    collect_directories("red4ext/plugins", "red4ext-plugin")
    collect_directories("bin/x64/plugins/cyber_engine_tweaks/mods", "cet-mod")
    table.sort(mods, function(a,b) return a.relative_path < b.relative_path end)

    local candidates = {
        { "CET", "bin/x64/plugins/cyber_engine_tweaks.asi", "file" },
        { "CET", "bin/x64/plugins/cyber_engine_tweaks", "directory" },
        { "RED4ext", "red4ext/RED4ext.dll", "file" },
        { "RED4ext", "red4ext/plugins", "directory" },
        { "redscript", "r6/scripts", "directory" },
        { "ArchiveXL", "red4ext/plugins/ArchiveXL/ArchiveXL.dll", "file" },
        { "ArchiveXL", "red4ext/plugins/ArchiveXL", "directory" },
    }
    for _, entry in ipairs(candidates) do
        if present(entry[2], entry[3]) then
            table.insert(signals, {
                name = entry[1], relative_path = entry[2], kind = entry[3]
            })
        end
    end
    table.sort(signals, function(a,b)
        if a.name == b.name then return a.relative_path < b.relative_path end
        return a.name < b.name
    end)
    return {
        mods = mods, framework_signals = signals, incomplete = incomplete,
        redmod_available = present("tools/redmod/bin/redMod.exe", "file")
    }
end

function policy.doctor(report)
    local findings = {}
    local function add(code, severity, message)
        table.insert(findings, {code = code, severity = severity, message = message})
    end
    if report.incomplete then
        add("SCAN_INCOMPLETE", "warning",
            "Inventory is incomplete because the scan entry limit was reached.")
    end
    if not report.redmod_available then
        add("REDMOD_NOT_FOUND", "info",
            "REDmod tools were not detected. REDmod deployment is unavailable.")
    end
    local unmanaged = false
    local redmod_content = false
    for _, item in ipairs(report.mods) do
        if not item.managed then unmanaged = true end
        if item.kind == "redmod-directory" then redmod_content = true end
    end
    if not report.redmod_available and redmod_content then
        add("REDMOD_CONTENT_WITHOUT_TOOLS", "warning",
            "REDmod-style directories exist, but redMod.exe was not found.")
    end
    if unmanaged then
        add("UNMANAGED_CONTENT", "warning",
            "Some detected mods are not owned by this manager; no changes will be made.")
    end
    for _, status in ipairs({"missing", "modified", "unsafe", "unreadable"}) do
        local count = 0
        for _, file in ipairs(report.owned_files) do
            if file.status == status then count += 1 end
        end
        if count > 0 then
            add("OWNED_FILE_" .. string.upper(status), "warning",
                string.format("%d manager-owned file(s) are %s; inspect scan ownership details before uninstalling.", count, status))
        end
    end
    return findings
end

return policy
