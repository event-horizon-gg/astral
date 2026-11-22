local _, Info = ...
local widget, label, background, elapsed = nil, nil, nil, 0
local values = {}
local function number(value)
    value = Info.Number(value)
    return value and value >= 0 and value or nil
end
local function plain(value)
    return value and tostring(math.floor(value + 0.5)) or '--'
end
local function colored(value, fps)
    local color = 'f2f1ed'
    if value then
        if fps and value < 30 or not fps and value >= 300 then color = 'ff9b93'
        elseif fps and value < 50 or not fps and value >= 150 then color = 'e6bd7b' end
    end
    return '|cff' .. color .. plain(value) .. '|r'
end
local function read()
    values.fps, values.home, values.world = nil, nil, nil
    if type(GetFramerate) == 'function' then
        local ok, fps = pcall(GetFramerate)
        if ok then values.fps = number(fps) end
    end
    if type(GetNetStats) == 'function' then
        local ok, bandwidthIn, bandwidthOut, home, world = pcall(GetNetStats)
        if ok then values.home, values.world = number(home), number(world) end
    end
end
local function tooltip()
    if not GameTooltip then return end
    GameTooltip:SetOwner(widget, 'ANCHOR_BOTTOMLEFT')
    GameTooltip:SetText('Astral Info', 0.61, 0.71, 1)
    GameTooltip:AddDoubleLine('FPS', plain(values.fps), 0.8, 0.8, 0.8, 1, 1, 1)
    GameTooltip:AddDoubleLine('Latency (home)', plain(values.home) .. ' ms', 0.8, 0.8, 0.8, 1, 1, 1)
    GameTooltip:AddDoubleLine('Latency (world)', plain(values.world) .. ' ms', 0.8, 0.8, 0.8, 1, 1, 1)
    if not Info.db.locked then GameTooltip:AddLine('Drag to move. /ainfo lock when finished.', 0.7, 0.7, 0.7, true) end
    GameTooltip:Show()
end
local function update()
    if not widget or not Info.db then return end
    read()
    local mode = Info.db.latency
    local latency = mode == 'both'
        and ('H ' .. colored(values.home) .. ' / W ' .. colored(values.world) .. ' ms')
        or (colored(values[mode]) .. ' ms')
    label:SetText(colored(values.fps, true) .. ' FPS  |cff777b85·|r  ' .. latency)
    widget:SetSize(math.max(100, label:GetStringWidth() + 18), Info.db.size + 14)
    if widget:IsMouseOver() and GameTooltip and GameTooltip:IsOwned(widget) then tooltip() end
end

function Info.ApplyPerformance()
    if not widget then return end
    local db = Info.db
    label:SetFont(STANDARD_TEXT_FONT or 'Fonts\\FRIZQT__.TTF', db.size, 'OUTLINE')
    widget:ClearAllPoints()
    widget:SetPoint(db.point, UIParent, db.relativePoint, db.x, db.y)
    background:SetAlpha(db.locked and 0 or 0.8)
    widget:SetShown(not db.hidden)
    if not db.hidden then update() end
end

function Info.CreatePerformance()
    if widget then return end
    widget = CreateFrame('Frame', 'AstralInfoFrame', UIParent)
    widget:SetFrameStrata('LOW')
    widget:SetClampedToScreen(true)
    widget:SetMovable(true)
    widget:EnableMouse(true)
    widget:RegisterForDrag('LeftButton')
    background = widget:CreateTexture(nil, 'BACKGROUND')
    background:SetAllPoints()
    if background.SetColorTexture then background:SetColorTexture(0.06, 0.07, 0.08, 1)
    else background:SetTexture(0.06, 0.07, 0.08, 1) end
    label = widget:CreateFontString(nil, 'OVERLAY')
    label:SetPoint('CENTER')
    label:SetShadowColor(0, 0, 0, 0.8)
    label:SetShadowOffset(0, -1)
    widget:SetScript('OnEnter', tooltip)
    widget:SetScript('OnLeave', function() if GameTooltip and GameTooltip:IsOwned(widget) then GameTooltip:Hide() end end)
    widget:SetScript('OnHide', function() if GameTooltip and GameTooltip:IsOwned(widget) then GameTooltip:Hide() end end)
    widget:SetScript('OnDragStart', function(self) if not Info.db.locked then self:StartMoving() end end)
    widget:SetScript('OnDragStop', function(self)
        self:StopMovingOrSizing()
        local point, _, relativePoint, x, y = self:GetPoint()
        Info.db.point, Info.db.relativePoint, Info.db.x, Info.db.y = point, relativePoint, x, y
    end)
    widget:SetScript('OnUpdate', function(_, delta)
        elapsed = elapsed + delta
        if elapsed >= 1 then elapsed = elapsed % 1; update() end
    end)
    Info.ApplyPerformance()
end
