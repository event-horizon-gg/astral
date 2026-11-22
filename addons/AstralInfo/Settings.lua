-- MIT License. Copyright (c) 2026 Joao Seixas.
local _, Info = ...
local points = { TOP = true, TOPLEFT = true, TOPRIGHT = true, CENTER = true,
    LEFT = true, RIGHT = true, BOTTOM = true, BOTTOMLEFT = true, BOTTOMRIGHT = true }
Info.defaults = { size = 12, point = 'TOPRIGHT', relativePoint = 'TOPRIGHT', x = -24, y = -220,
    locked = true, hidden = false, latency = 'world', mounts = true, source = true }

function Info.Public(value)
    if issecretvalue and issecretvalue(value) then return nil end
    return value
end

function Info.Number(value)
    value = Info.Public(value)
    if type(value) ~= 'number' or value ~= value or value == math.huge or value == -math.huge then return nil end
    return value
end

function Info.Configure(saved)
    local db = type(saved) == 'table' and saved or {}
    for key, value in pairs(Info.defaults) do
        if type(db[key]) ~= type(value) then db[key] = value end
    end
    db.size = math.max(10, math.min(30, Info.Number(db.size) or 12))
    db.point = points[db.point] and db.point or 'TOPRIGHT'
    db.relativePoint = points[db.relativePoint] and db.relativePoint or 'TOPRIGHT'
    db.x, db.y = Info.Number(db.x) or -24, Info.Number(db.y) or -220
    if db.latency ~= 'home' and db.latency ~= 'world' and db.latency ~= 'both' then db.latency = 'world' end
    return db
end

function Info.Say(text)
    print('|cff9cb6ffAstral Info:|r ' .. text)
end
