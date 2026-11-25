local _, Info = ...
local events = CreateFrame('Frame')
local function initialize()
    if Info.db then return end
    AstralInfoDB = Info.Configure(AstralInfoDB)
    Info.db = AstralInfoDB
    Info.CreatePerformance()
    Info.InstallMountTooltip()
    events:UnregisterEvent('PLAYER_LOGIN')
    pcall(events.RegisterEvent, events, 'NEW_MOUNT_ADDED')
    pcall(events.RegisterEvent, events, 'PLAYER_ENTERING_WORLD')
end
events:RegisterEvent('PLAYER_LOGIN')
events:SetScript('OnEvent', function(_, event)
    if event == 'PLAYER_LOGIN' then initialize() else Info.InvalidateMounts() end
end)
if IsLoggedIn and IsLoggedIn() then initialize() end

SLASH_ASTRALINFO1 = '/astralinfo'
SLASH_ASTRALINFO2 = '/ainfo'
SlashCmdList.ASTRALINFO = function(message)
    if not Info.db then return end
    local command, argument = message:lower():match('^%s*(%S*)%s*(.-)%s*$')
    local db = Info.db
    if command == 'unlock' then db.locked = false; Info.Say('Drag the bar, then /ainfo lock.')
    elseif command == 'lock' then db.locked = true; Info.Say('Position locked.')
    elseif command == 'show' then db.hidden = false
    elseif command == 'hide' then db.hidden = true
    elseif command == 'size' and tonumber(argument) then db.size = math.max(10, math.min(30, tonumber(argument)))
    elseif command == 'latency' and (argument == 'home' or argument == 'world' or argument == 'both') then db.latency = argument
    elseif command == 'mounts' and (argument == 'on' or argument == 'off') then db.mounts = argument == 'on'; Info.Say('Mount tooltips ' .. argument .. '.')
    elseif command == 'source' and (argument == 'on' or argument == 'off') then db.source = argument == 'on'
    elseif command == 'reset' then
        for key, value in pairs(Info.defaults) do db[key] = value end
    else Info.Say('/ainfo unlock, lock, size 12, latency home|world|both, mounts on|off, source on|off, hide, show, reset'); return end
    Info.ApplyPerformance()
end
