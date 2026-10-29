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

local function listing(folder, required)
    local ok, response = pcall(dir.list, 1, folder)
    if ok then return response end
    -- Missing layout roots are optional; discovered descendants must be readable.
    if required then return { items = {}, incomplete = true } end
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
            local response = listing(current, current ~= folder)
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


-- Schema-v1 source-to-destination proposals. The host validates every operation.
function policy.plan(files)
    local operations = {}
    local blockers = {}
    local redmods = {}
    local function block(code, message)
        table.insert(blockers, { code = code, message = message })
    end
    local function kind_of(parts, i)
        local count = #parts - i + 1
        local a = string.lower(parts[i] or "")
        local b = string.lower(parts[i + 1] or "")
        local c = string.lower(parts[i + 2] or "")
        local d = string.lower(parts[i + 3] or "")
        local e = string.lower(parts[i + 4] or "")
        local filename = string.lower(parts[#parts] or "")
        if count == 4 and a == "archive" and b == "pc" and c == "mod"
            and string.sub(filename, -8) == ".archive" then
            return "legacy-archive"
        end
        if count >= 3 and a == "mods" and b ~= "" then return "redmod" end
        if count >= 3 and a == "r6" and b == "scripts" then return "redscript" end
        if count >= 3 and a == "r6" and b == "tweaks" then return "tweak" end
        if count >= 7 and a == "bin" and b == "x64" and c == "plugins"
            and d == "cyber_engine_tweaks" and e == "mods"
            and parts[i + 5] ~= "" then return "cet" end
        if count >= 4 and a == "red4ext" and b == "plugins"
            and c ~= "" then return "red4ext" end
        return nil
    end
    local function is_documentation(path)
        local name = string.lower(string.match(path, "[^/]+$") or "")
        local stem = string.match(name, "^([^%.]+)") or name
        if name == "license" or name == "copying"
            or name == "readme" or name == "changelog"
            or name == "description.txt" then return true end
        local ext = string.match(name, "%.([^%.]+)$") or ""
        return (ext == "txt" or ext == "md" or ext == "rst")
            and (string.sub(stem, 1, 6) == "readme"
                or string.sub(stem, 1, 7) == "license"
                or string.sub(stem, 1, 9) == "changelog")
    end
    for _, file in ipairs(files) do
        local parts = {}
        for item in string.gmatch(file.source, "[^/]+") do
            table.insert(parts, item)
        end
        local mapped = false
        for i = 1, #parts do
            local kind = kind_of(parts, i)
            if kind ~= nil then
                local wrapper = table.concat(parts, "/", 1, i - 1)
                local target = table.concat(parts, "/", i)
                table.insert(operations, {
                    source = file.source, destination = target, kind = kind,
                    package_root = wrapper,
                })
                -- Extended layouts remain preview-only until their own milestones.
                if kind ~= "legacy-archive" then
                    block("NOT_INSTALLABLE",
                        "ZIP installation currently supports legacy .archive content only")
                end
                if kind == "redmod" then
                    local name = string.lower(parts[i + 1])
                    if redmods[name] == nil then redmods[name] = false end
                    if #parts - i + 1 == 3
                        and string.lower(parts[i + 2]) == "info.json" then
                        redmods[name] = true
                    end
                end
                mapped = true
                break
            end
        end
        if not mapped and not is_documentation(file.source) then
            block("UNRECOGNIZED_LAYOUT", "Unknown mod layout: " .. file.source)
        end
    end
    local redmod_found = false
    for name, has_info in pairs(redmods) do
        redmod_found = true
        if not has_info then
            block("REDMOD_INFO_MISSING", "REDmod has no info.json: " .. name)
        end
    end
    if redmod_found and not present("tools/redmod/bin/redMod.exe", "file") then
        block("REDMOD_NOT_INSTALLED", "REDmod deployment tools are not installed")
    end
    return { schema_version = 1, operations = operations, blockers = blockers }
end

return policy
