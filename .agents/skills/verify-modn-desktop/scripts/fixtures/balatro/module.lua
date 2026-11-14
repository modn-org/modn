-- Fixture Balatro policy for the Modn Desktop verifier.
--
-- This module ports the layout rules that the desktop journeys need from the
-- v1 Balatro module: the `Balatro.exe` marker, the `Balatro/Mods` content
-- root, the Steam app identity and the executable launch target. It is small
-- on purpose. Journeys use it only for detection, remove, play and a local ZIP
-- install; it is not a compatibility claim for the real game.

local fs = mods_host.fs
local dir = mods_host.directory

local policy = {
    game_id = "balatro",
    api_version = mods_host.api_version,
    marker = "Balatro.exe",
}

local function metadata(root_id, relative)
    local ok, value = pcall(fs.metadata, root_id, relative)
    if ok then return value end
    return nil
end

local function present(root_id, relative, kind)
    local item = metadata(root_id, relative)
    return item ~= nil and item.kind == kind
end

function policy.validate()
    return present(1, policy.marker, "file")
end

local function lower(value)
    return string.lower(value or "")
end

local function extension(value)
    return string.match(lower(value), "%.([^%.]+)$") or ""
end

local function is_lua(value)
    return extension(value) == "lua"
end

local function is_documentation(value)
    local ext = extension(value)
    return ext == "txt" or ext == "md" or ext == "rst"
end

local function split_path(value)
    local parts = {}
    for part in string.gmatch(value, "[^/]+") do table.insert(parts, part) end
    return parts
end

local function is_save_or_settings(relative)
    local parts = split_path(relative)
    if parts[1] == "1" or parts[1] == "2" or parts[1] == "3" then return true end
    for _, part in ipairs(parts) do
        local name = lower(part)
        if string.sub(name, -4) == ".jkr" or string.sub(name, 1, 8) == "settings" then
            return true
        end
    end
    return false
end

local function is_executable_payload(relative)
    local ext = extension(relative)
    return ext == "exe" or ext == "bat" or ext == "dll" or ext == "cmd"
        or ext == "com" or ext == "scr" or ext == "ps1" or ext == "vbs"
        or ext == "msi" or ext == "lnk" or ext == "url"
end

local function all_under(files, top)
    local prefix = top .. "/"
    for _, file in ipairs(files) do
        if string.sub(file.source, 1, #prefix) ~= prefix then return false end
    end
    return true
end

local function relative_to(files, top)
    local prefix_length = #top + 2
    local result = {}
    for _, file in ipairs(files) do
        table.insert(result, {
            source = string.sub(file.source, prefix_length),
            bytes = file.bytes,
        })
    end
    return result
end

local function without_documentation(files)
    local result = {}
    for _, file in ipairs(files) do
        if not is_documentation(file.source) then table.insert(result, file) end
    end
    return result
end

function policy.plan(files)
    local operations = {}
    local blockers = {}

    local function block(code, message)
        table.insert(blockers, { code = code, message = message })
    end

    if #files == 0 then
        block("EMPTY_PACKAGE", "Archive contains no files")
        return { schema_version = 1, operations = operations, blockers = blockers }
    end

    for _, file in ipairs(files) do
        if is_save_or_settings(file.source) then
            block("SAVE_DATA_PAYLOAD", "Save or settings payload is not installed: " .. file.source)
        end
    end
    if #blockers > 0 then
        return { schema_version = 1, operations = operations, blockers = blockers }
    end

    local relative = files
    local prefix = ""
    local first_top = string.match(files[1].source, "^([^/]+)")
    if first_top ~= nil and lower(first_top) == "mods" and all_under(files, first_top) then
        relative = relative_to(files, first_top)
        prefix = first_top
    end

    local content = without_documentation(relative)
    local top = content[1] and string.match(content[1].source, "^([^/]+)") or nil
    local folder = nil
    if top ~= nil and all_under(content, top) then
        local parts = split_path(top)
        if #parts == 1 and not is_lua(top) then folder = top end
    end

    local native_only = true
    for _, file in ipairs(content) do
        if string.find(file.source, "/", 1, true) ~= nil then
            native_only = false
            break
        end
        if lower(file.source) ~= "winmm.dll" and not is_documentation(file.source) then
            native_only = false
            break
        end
    end
    if native_only then
        for _, file in ipairs(content) do
            if lower(file.source) == "winmm.dll" then
                table.insert(operations, {
                    source = prefix == "" and file.source or prefix .. "/" .. file.source,
                    destination = "winmm.dll",
                    kind = "native-library",
                    package_root = prefix,
                })
            end
        end
        if #operations == 0 then
            block("NO_INSTALLABLE_FILES", "No installable files")
        end
        return { schema_version = 1, operations = operations, blockers = blockers }
    end

    for _, file in ipairs(content) do
        local path = file.source
        local source = prefix == "" and path or prefix .. "/" .. path
        local destination, kind, package_root
        if folder ~= nil then
            destination = prefix == "" and path or prefix .. "/" .. path
            kind = lower(folder) == "smods" and "steamodded-file" or "balatro-mod-file"
            package_root = prefix == "" and folder or prefix .. "/" .. folder
        elseif is_lua(path) and string.find(path, "/", 1, true) == nil then
            destination = path
            kind = "balatro-mod-file"
            package_root = prefix
        else
            block("UNRECOGNIZED_LAYOUT", "Unsupported Balatro package layout: " .. source)
            destination = nil
        end
        if destination ~= nil then
            if is_executable_payload(path) then
                block("EXECUTABLE_PAYLOAD", "Executable payload is not supported: " .. destination)
            else
                table.insert(operations, {
                    source = source,
                    destination = destination,
                    kind = kind,
                    package_root = package_root,
                    root = "content",
                })
            end
        end
    end

    if #operations == 0 and #blockers == 0 then
        block("NO_INSTALLABLE_FILES", "No installable files")
    end
    return { schema_version = 1, operations = operations, blockers = blockers }
end

local function note_listing(response, issues)
    if not response.incomplete then return false end
    if response.truncated then issues.SCAN_INCOMPLETE = true end
    for _ in ipairs(response.unsafe_names or {}) do
        issues.SCAN_UNSAFE_ENTRY = true
    end
    if response.unreadable then issues.SCAN_UNREADABLE_ENTRY = true end
    if not (response.truncated or response.unreadable
        or (response.unsafe_names and #response.unsafe_names > 0)) then
        issues.SCAN_INCOMPLETE = true
    end
    return true
end

local function safe_name(name)
    return type(name) == "string" and name ~= "" and name ~= "." and name ~= ".."
        and string.find(name, "[/\\]") == nil
end

function policy.scan(limit)
    local mods = {}
    local signals = {}
    local issues = {}
    local incomplete = false

    if present(1, "winmm.dll", "file") then
        table.insert(signals, { name = "Lovely", relative_path = "winmm.dll", kind = "file" })
    end

    local roots = mods_host.discovery.roots()
    if #roots >= 2 then
        local function push(relative, kind)
            table.insert(mods, { relative_path = relative, kind = kind, root = "content" })
        end
        local function walk_smods()
            local stack = { "smods" }
            while #stack > 0 do
                local folder = table.remove(stack)
                local listed, response = pcall(dir.list, 2, folder, math.max(limit - #mods, 1))
                if not listed then
                    incomplete = true
                    issues.SCAN_UNREADABLE_ENTRY = true
                else
                    if note_listing(response, issues) then incomplete = true end
                    for _, entry in ipairs(response.items) do
                        if #mods >= limit then
                            incomplete = true
                            issues.SCAN_INCOMPLETE = true
                            return
                        end
                        if not safe_name(entry.name) then
                            incomplete = true
                            issues.SCAN_UNSAFE_ENTRY = true
                        else
                            local relative = folder .. "/" .. entry.name
                            if entry.kind == "directory" then
                                table.insert(stack, relative)
                            elseif entry.kind == "file" then
                                push(relative, "steamodded")
                            end
                        end
                    end
                end
            end
        end
        local ok, top = pcall(dir.list, 2, "", math.max(limit, 1))
        if ok then
            if note_listing(top, issues) then incomplete = true end
            for _, entry in ipairs(top.items) do
                if #mods >= limit then
                    incomplete = true
                    issues.SCAN_INCOMPLETE = true
                    break
                end
                if not safe_name(entry.name) then
                    incomplete = true
                    issues.SCAN_UNSAFE_ENTRY = true
                elseif entry.kind == "file" then
                    if is_lua(entry.name) then push(entry.name, "single") end
                elseif entry.kind == "directory" then
                    if lower(entry.name) == "smods" then
                        push(entry.name, "steamodded")
                        walk_smods()
                    else
                        push(entry.name, "mod")
                    end
                end
            end
        end
    end

    local scan_issues = {}
    for code in pairs(issues) do table.insert(scan_issues, code) end
    table.sort(scan_issues)
    table.sort(mods, function(a, b) return a.relative_path < b.relative_path end)
    return {
        mods = mods, framework_signals = signals, incomplete = incomplete,
        scan_issues = scan_issues, redmod_available = false,
    }
end

function policy.doctor(report)
    local findings = {}
    local function add(code, severity, message, action)
        table.insert(findings, {
            code = code, severity = severity, message = message, action = action,
        })
    end

    local has_winmm = false
    for _, signal in ipairs(report.framework_signals or {}) do
        if signal.relative_path == "winmm.dll" then has_winmm = true end
    end
    if has_winmm then
        add("LOVELY_PRESENT", "info", "Lovely injector detected (winmm.dll).")
    else
        add("LOVELY_MISSING", "warning",
            "Lovely injector not detected; mods in the Balatro Mods folder will not load.",
            "Install Lovely so winmm.dll sits beside Balatro.exe, then run `modn doctor` again.")
    end

    for _, code in ipairs(report.scan_issues or {}) do
        if code == "SCAN_INCOMPLETE" then
            add(code, "warning", "Inventory truncated by the scan entry limit.")
        elseif code == "SCAN_UNSAFE_ENTRY" then
            add(code, "warning", "Unsafe or linked directory entries were skipped.")
        elseif code == "SCAN_UNREADABLE_ENTRY" then
            add(code, "warning", "Unreadable directory entries were skipped.")
        end
    end

    for _, item in ipairs(report.mods or {}) do
        if not item.managed then
            add("UNMANAGED_CONTENT", "warning",
                "Some detected Balatro mods are not owned by this manager; no changes will be made.")
            break
        end
    end

    return findings
end

return policy
