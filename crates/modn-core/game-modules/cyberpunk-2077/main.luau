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
local function listing(folder, required, max_items)
    local ok, response = pcall(dir.list, 1, folder, max_items)
    if ok then return response end
    -- Missing layout roots are optional; discovered descendants must be readable.
    if required then return { items = {}, incomplete = true } end
    return nil
end
function policy.scan(limit)
    local mods = {}
    local signals = {}
    local remaining = limit
    local framework_remaining = 256
    local incomplete = false
    local issues = {}
    local function mark_limit()
        incomplete = true
        issues.SCAN_INCOMPLETE = true
    end
    local function note_listing(response, folder)
        if not response.incomplete then return end
        for _, entry in ipairs(response.unsafe_names or {}) do
            local name = entry.name
            if folder == "red4ext/plugins/ArchiveXL" and name == "ArchiveXL.log"
                and entry.file_symlink and present("red4ext/plugins/ArchiveXL/ArchiveXL.dll", "file") then
                -- Generated log is not mod content; never follow its link.
                issues.SCAN_IGNORED_GENERATED_LOG = true
            else
                incomplete = true
                issues.SCAN_UNSAFE_ENTRY = true
            end
        end
        if response.truncated then mark_limit() end
        if response.unreadable or response.truncated == nil then
            incomplete = true
            issues.SCAN_UNREADABLE_ENTRY = true
        end
    end
    local function push(relative, kind)
        table.insert(mods, { relative_path = relative, kind = kind })
    end
    local function consume(framework)
        if remaining <= 0 or (framework and framework_remaining <= 0) then
            mark_limit()
            return false
        end
        remaining -= 1
        if framework then framework_remaining -= 1 end
        return true
    end
    local function collect_files(folder, extensions, kind, required_root)
        local framework = kind == "cet-mod" or kind == "red4ext-plugin"
            or kind == "redscript" or kind == "redscript-hint" or kind == "tweak" or kind == "input-xml"
        local stack = { folder }
        local empty_peeks = 0
        while #stack > 0 do
            local current = table.remove(stack)
            if framework and framework_remaining <= 0 then
                -- Bounded peeks: uncertainty must be reported as incomplete.
                empty_peeks += 1
                if empty_peeks > 32 then
                    mark_limit()
                    return
                end
                local peek = listing(current, current ~= folder or required_root, 1)
                if peek ~= nil and (peek.incomplete or #peek.items > 0) then
                    note_listing(peek, current)
                    mark_limit()
                    return
                end
            else
                local max_items = framework and framework_remaining or nil
                local response = listing(current, current ~= folder or required_root, max_items)
                if response ~= nil then
                    note_listing(response, current)
                    for _, entry in ipairs(response.items) do
                        if not consume(framework) then return end
                        local relative = current .. "/" .. entry.name
                        if entry.kind == "directory" then
                            if kind ~= "input-xml" then table.insert(stack, relative) end
                        elseif entry.kind == "file" then
                            local ext = string.match(entry.name, "%.([^%.]+)$")
                            if extensions == nil or (ext ~= nil and extensions[string.lower(ext)]) then
                                push(relative, kind == "legacy-archive" and string.lower(ext) == "xl"
                                and "archive-xl" or kind)
                            end
                        end
                    end
                end
            end
        end
    end
    local function collect_directories(folder, kind, scan_files)
        local exhausted = scan_files and framework_remaining <= 0
        local max_items = scan_files and math.max(framework_remaining, 1) or nil
        local response = listing(folder, nil, max_items)
        if response == nil then return end
        if exhausted then
            if response.incomplete or #response.items > 0 then
                note_listing(response, folder)
                mark_limit()
            end
            return
        end
        note_listing(response, folder)
        for _, entry in ipairs(response.items) do
            if not consume(scan_files) then return end
            if entry.kind == "directory" then
                local subfolder = folder .. "/" .. entry.name
                if scan_files then
                    -- Emit individual files for CET/RED4ext ownership.
                    local start = #mods
                    collect_files(subfolder, nil, kind, true)
                    if #mods == start and framework_remaining > 0 then
                        push(subfolder, kind)
                    end
                else
                    -- REDmod remains a directory-only presence signal.
                    push(subfolder, kind)
                end
            end
        end
    end
    collect_files("archive/pc/mod", { archive = true, xl = true }, "legacy-archive")
    collect_files("r6/scripts", { reds = true }, "redscript")
    collect_files("r6/config/redsUserHints", { toml = true }, "redscript-hint")
    collect_files("r6/tweaks", { yaml = true, yml = true }, "tweak")
    collect_files("r6/input", { xml = true }, "input-xml")
    collect_directories("mods", "redmod-directory", false)
    collect_directories("red4ext/plugins", "red4ext-plugin", true)
    collect_directories("bin/x64/plugins/cyber_engine_tweaks/mods", "cet-mod", true)
    table.sort(mods, function(a,b) return a.relative_path < b.relative_path end)
    local candidates = {
        { "CET", "bin/x64/plugins/cyber_engine_tweaks.asi", "file" },
        { "CET", "bin/x64/plugins/cyber_engine_tweaks", "directory" },
        { "RED4ext", "red4ext/RED4ext.dll", "file" },
        { "RED4ext", "red4ext/plugins", "directory" },
        { "redscript", "r6/scripts", "directory" },
        { "ArchiveXL", "red4ext/plugins/ArchiveXL/ArchiveXL.dll", "file" },
        { "ArchiveXL", "red4ext/plugins/ArchiveXL", "directory" },
        { "redscript compiler", "engine/tools/scc.exe", "file" },
        { "TweakXL", "red4ext/plugins/TweakXL/TweakXL.dll", "file" }, { "Input Loader", "red4ext/plugins/input_loader/input_loader.dll", "file" },
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
    local scan_issues = {}
    for code in pairs(issues) do table.insert(scan_issues, code) end
    table.sort(scan_issues)
    return {
        mods = mods, framework_signals = signals, incomplete = incomplete,
        scan_issues = scan_issues,
        redmod_available = present("tools/redmod/bin/redMod.exe", "file")
    }
end
function policy.doctor(report)
    local findings = {}
    local function add(code, severity, message)
        table.insert(findings, {code = code, severity = severity, message = message})
    end
    for _, code in ipairs(report.scan_issues or {}) do
        if code == "SCAN_INCOMPLETE" then
            add(code, "warning", "Inventory truncated by the scan entry limit.")
        elseif code == "SCAN_UNSAFE_ENTRY" then
            add(code, "warning", "Unsafe or linked directory entries were skipped.")
        elseif code == "SCAN_UNREADABLE_ENTRY" then
            add(code, "warning", "Unreadable directory entries were skipped.")
        elseif code == "SCAN_IGNORED_GENERATED_LOG" then
            add(code, "info", "Generated ArchiveXL.log link was ignored; adoption is blocked.")
        end
    end
    if report.incomplete and #(report.scan_issues or {}) == 0 then
        add("SCAN_INCOMPLETE", "warning", "Inventory is incomplete; some content was not inspected.")
    end
    if not report.redmod_available then
        add("REDMOD_NOT_FOUND", "info", "REDmod tools were not detected. REDmod deployment is unavailable.")
    end
    local unmanaged = false
    local redmod_content = false
    local needs_redscript, needs_archivexl, needs_tweakxl = false, false, false
    local needs_input_loader = false
    for _, item in ipairs(report.mods) do
        if item.kind == "input-xml" then needs_input_loader = true end
        if item.kind == "redscript" or item.kind == "redscript-hint" then
            needs_redscript = true
        elseif item.kind == "archive-xl" then
            needs_archivexl = true
        elseif item.kind == "tweak" then
            needs_tweakxl = true
        end
        if not item.managed then unmanaged = true end
        if item.kind == "redmod-directory" then redmod_content = true end
    end
    local function has_signal(relative)
        for _, signal in ipairs(report.framework_signals) do
            if signal.relative_path == relative then return true end
        end
        return false
    end
    if needs_input_loader and not has_signal("red4ext/plugins/input_loader/input_loader.dll") then
        add("INPUT_LOADER_NOT_INSTALLED", "warning", "Input Loader is required for r6/input XML.")
    end
    if needs_redscript and not has_signal("engine/tools/scc.exe") then add("REDSCRIPT_NOT_INSTALLED", "warning", "REDscript compiler not detected; script loading is not verified.") end
    if needs_archivexl and not has_signal("red4ext/plugins/ArchiveXL/ArchiveXL.dll") then add("ARCHIVEXL_NOT_INSTALLED", "warning", "ArchiveXL plugin not detected; XL content loading is not verified.") end
    if needs_tweakxl and not has_signal("red4ext/plugins/TweakXL/TweakXL.dll") then add("TWEAKXL_NOT_INSTALLED", "warning", "TweakXL plugin not detected; tweak loading is not verified.") end
    if not report.redmod_available and redmod_content then add("REDMOD_CONTENT_WITHOUT_TOOLS", "warning", "REDmod-style directories exist, but redMod.exe was not found.") end
    if unmanaged then add("UNMANAGED_CONTENT", "warning", "Some detected mods are not owned by this manager; no changes will be made.") end
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
function policy.plan(files)
    local operations = {}
    local blockers = {}
    local ignored_generated = {}
    local redmods = {}
    local generated = { ["engine/config/platform/pc/input_loader.ini"] = true, ["r6/cache/inputContexts.xml"] = true, ["r6/cache/inputUserMappings.xml"] = true }
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
        if count == 4 and a == "archive" and b == "pc" and c == "mod" then
            if string.sub(filename, -8) == ".archive" then return "legacy-archive" end
            return "archive-xl"
        end
        if count >= 3 and a == "mods" and b ~= "" then return "redmod" end
        if count >= 3 and a == "r6" and b == "scripts" then return "redscript" end
        if count >= 3 and a == "r6" and b == "tweaks" then return "tweak" end
        if count == 3 and a == "r6" and b == "input" then return "input-xml" end
        if count == 4 and a == "r6" and b == "config" and c == "redsuserhints" then
            return "redscript-hint"
        end
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
    local canonical_framework_prefixes = {
        cet = { "bin", "x64", "plugins", "cyber_engine_tweaks", "mods" },
        red4ext = { "red4ext", "plugins" },
        redscript = { "r6", "scripts" },
        tweak = { "r6", "tweaks" },
        ["redscript-hint"] = { "r6", "config", "redsUserHints" },
        ["input-xml"] = { "r6", "input" },
        ["legacy-archive"] = { "archive", "pc", "mod" },
        ["archive-xl"] = { "archive", "pc", "mod" },
    }
    local allowed_extensions = {
        redscript = { reds = true },
        ["input-xml"] = { xml = true },
        tweak = { yaml = true, yml = true },
        ["redscript-hint"] = { toml = true },
        ["archive-xl"] = { xl = true },
    }
    local loader_roots = {}
    for _, file in ipairs(files) do
        local parts = {}
        for part in string.gmatch(file.source, "[^/]+") do table.insert(parts, part) end
        for i = 1, #parts do
            local kind = kind_of(parts, i)
            if kind ~= nil then
                if kind == "red4ext" and string.lower(table.concat(parts, "/", i))
                    == "red4ext/plugins/input_loader/input_loader.dll" then
                    loader_roots[table.concat(parts, "/", 1, i - 1)] = true
                end
                break
            end
        end
    end
    local needs_redscript, needs_archivexl, needs_tweakxl = false, false, false
    local needs_input_loader = false
    for _, file in ipairs(files) do
        local parts = {}
        for item in string.gmatch(file.source, "[^/]+") do
            table.insert(parts, item)
        end
        local mapped = false
        for wrapper in pairs(loader_roots) do
            local prefix = wrapper == "" and "" or wrapper .. "/"
            if string.sub(file.source, 1, #prefix) == prefix
                and generated[string.sub(file.source, #prefix + 1)] then
                table.insert(ignored_generated, file.source)
                mapped = true
                break
            end
        end
        if not mapped then for i = 1, #parts do
            local kind = kind_of(parts, i)
            if kind ~= nil then
                if is_documentation(file.source) then
                    mapped = true
                    break
                end
                local expected = canonical_framework_prefixes[kind]
                if expected ~= nil then
                    for offset, component in ipairs(expected) do
                        if parts[i + offset - 1] ~= component then
                            block("NONCANONICAL_LAYOUT",
                                "Framework directory casing differs: " .. file.source)
                            break
                        end
                    end
                end
                local wrapper = table.concat(parts, "/", 1, i - 1)
                local target = table.concat(parts, "/", i)
                table.insert(operations, {
                    source = file.source, destination = target, kind = kind,
                    package_root = wrapper,
                })
                local allowed = allowed_extensions[kind]
                local extension = string.match(string.lower(parts[#parts]), "%.([^%.]+)$") or ""
                if allowed ~= nil and not allowed[extension] then
                    block("UNSUPPORTED_CONTENT", "Unsupported file in " .. kind .. ": " .. file.source)
                end
                if kind == "redmod" then
                    block("NOT_INSTALLABLE", "REDmod deployment is not supported")
                end
                if kind == "input-xml" then needs_input_loader = true end
                if kind == "redscript" or kind == "redscript-hint" then
                    needs_redscript = true
                elseif kind == "archive-xl" then
                    needs_archivexl = true
                elseif kind == "tweak" then
                    needs_tweakxl = true
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
        end end
        if not mapped and not is_documentation(file.source) then
            block("UNRECOGNIZED_LAYOUT", "Unknown mod layout: " .. file.source)
        end
    end
    if needs_input_loader and next(loader_roots) == nil
        and not present("red4ext/plugins/input_loader/input_loader.dll", "file") then
        block("INPUT_LOADER_NOT_INSTALLED", "Input Loader plugin is required for r6/input XML")
    end
    if needs_redscript and not present("engine/tools/scc.exe", "file") then
        block("REDSCRIPT_NOT_INSTALLED", "REDscript compiler was not detected (engine/tools/scc.exe)")
    end
    if needs_archivexl and not present("red4ext/plugins/ArchiveXL/ArchiveXL.dll", "file") then
        block("ARCHIVEXL_NOT_INSTALLED", "ArchiveXL plugin was not detected")
    end
    if needs_tweakxl and not present("red4ext/plugins/TweakXL/TweakXL.dll", "file") then
        block("TWEAKXL_NOT_INSTALLED", "TweakXL plugin was not detected")
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
    return { schema_version = 1, operations = operations, blockers = blockers,
        skipped_generated_files = ignored_generated }
end
-- Adopt only specific recognized files or a single plugin subtree.
function policy.adoption(mods)
    local groups = {}
    local rejected = {}
    local order = {}
    for _, item in ipairs(mods) do
        local path = item.relative_path
        local selector
        if item.kind == "cet-mod" then
            selector = string.match(path, "^(bin/x64/plugins/cyber_engine_tweaks/mods/[^/]+)/")
        elseif item.kind == "red4ext-plugin" then
            selector = string.match(path, "^(red4ext/plugins/[^/]+)/")
        elseif item.kind == "legacy-archive" or item.kind == "archive-xl"
            or item.kind == "redscript" or item.kind == "redscript-hint"
            or item.kind == "tweak" or item.kind == "input-xml" then
            selector = path
            if item.kind == "input-xml" and not string.match(path, "^r6/input/[^/]+%.xml$") then selector = nil end
        end
        -- User-generated configs, logs, caches and backups are ambiguous.
        local lower = string.lower(path)
        local unsafe = (item.kind ~= "redscript-hint"
            and string.find(lower, "/config/", 1, true))
            or string.find(lower, "/logs/", 1, true)
            or string.match(lower, "%.log$")
            or string.match(lower, "%.bak$")
            or string.match(lower, "%.tmp$")
            or string.match(lower, "%.json$")
        if selector ~= nil and unsafe then rejected[selector] = true end
        if selector ~= nil and not unsafe then
            if groups[selector] == nil then
                groups[selector] = { selector = selector, kind = item.kind, files = {} }
                table.insert(order, selector)
            end
            table.insert(groups[selector].files, path)
        end
    end
    table.sort(order)
    local output = {}
    for _, selector in ipairs(order) do
        table.sort(groups[selector].files)
        if not rejected[selector] then table.insert(output, groups[selector]) end
    end
    return { schema_version = 1, groups = output }
end
return policy
