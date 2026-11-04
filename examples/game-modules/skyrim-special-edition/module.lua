local fs = mods_host.fs
local directory = mods_host.directory
local policy = {
    game_id = "skyrim-special-edition", version = "0.1.0", api_version = 1,
    marker = "SkyrimSE.exe", common_paths = {},
}

local function metadata(path)
    return fs.metadata(1, path)
end

function policy.validate()
    local executable = metadata("SkyrimSE.exe")
    local data = metadata("Data")
    return executable ~= nil and executable.kind == "file"
        and data ~= nil and data.kind == "directory"
end

function policy.steam_name() return "Skyrim Special Edition" end
function policy.is_epic(_) return false end

local function supported(path)
    local lower = string.lower(path)
    if not string.find(lower, "/", 1, true) then
        return string.match(lower, "%.esp$") or string.match(lower, "%.esm$")
            or string.match(lower, "%.esl$") or string.match(lower, "%.bsa$")
    end
    return string.match(lower, "^meshes/.+%.nif$")
        or string.match(lower, "^textures/.+%.dds$")
end

function policy.plan(files)
    local operations, blockers = {}, {}
    for _, file in ipairs(files) do
        local source = file.source
        local relative = string.match(source, "^Data/(.+)$")
        if relative and supported(relative) then
            table.insert(operations, {
                source = source, destination = "Data/" .. relative,
                kind = "skyrim-data", package_root = "",
            })
        elseif string.lower(source) ~= "readme.txt" and string.lower(source) ~= "license.txt" then
            table.insert(blockers, {
                code = "UNSUPPORTED_SKYRIM_LAYOUT",
                message = "Use Data plugins, BSA archives, meshes or textures: " .. source,
            })
        end
    end
    return { schema_version = 1, operations = operations, blockers = blockers }
end

function policy.scan(limit)
    local mods, stack, visited = {}, { "Data" }, 0
    local incomplete, unsafe = false, false
    while #stack > 0 and not incomplete do
        local folder = table.remove(stack)
        local ok, result = pcall(directory.list, 1, folder)
        if not ok then
            incomplete = true
        else
            incomplete = result.incomplete
            if result.unsafe_names and #result.unsafe_names > 0 then unsafe = true end
            for _, entry in ipairs(result.items) do
                visited = visited + 1
                if visited > limit then incomplete = true; break end
                local path = folder .. "/" .. entry.name
                local relative = string.sub(path, 6)
                if entry.kind == "directory" then
                    local lower = string.lower(relative)
                    if lower == "meshes" or lower == "textures"
                        or string.match(lower, "^meshes/") or string.match(lower, "^textures/") then
                        table.insert(stack, path)
                    end
                elseif entry.kind == "file" and supported(relative) then
                    table.insert(mods, { relative_path = path, kind = "skyrim-data" })
                end
            end
        end
    end
    local issues = {}
    if incomplete then table.insert(issues, "SCAN_INCOMPLETE") end
    if unsafe then table.insert(issues, "SCAN_UNSAFE_ENTRY"); incomplete = true end
    return { mods = mods, framework_signals = {}, redmod_available = false,
        incomplete = incomplete, scan_issues = issues }
end

function policy.doctor(report)
    local result = {{ code = "SKYRIM_LOAD_ORDER_MANUAL", severity = "info",
        message = "Enable plugins and set load order with your chosen game tools.",
        action = "Check plugin requirements and load order before starting Skyrim." }}
    if report.incomplete then
        table.insert(result, { code = "SCAN_INCOMPLETE", severity = "warning",
            message = "Some Skyrim Data entries could not be inspected." })
    end
    for _, file in ipairs(report.owned_files) do
        if file.status ~= "intact" then
            table.insert(result, { code = "OWNED_FILE_DRIFT", severity = "warning",
                message = "A managed Skyrim file is missing or modified." })
            break
        end
    end
    return result
end

return policy
