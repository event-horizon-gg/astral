local _, Info = ...
local fallback, marked, adding, installed = nil, false, false, false
local function public(value) return Info.Public(value) end
local function blocked()
    return InCombatLockdown and InCombatLockdown()
end
local function journal()
    return C_MountJournal and type(C_MountJournal.GetMountInfoByID) == 'function'
end
function Info.InvalidateMounts() fallback = nil end
local function mountForSpell(spellID)
    if type(C_MountJournal.GetMountFromSpell) == 'function' then
        local ok, id = pcall(C_MountJournal.GetMountFromSpell, spellID)
        return ok and Info.Number(id) or nil
    end
    if not fallback then
        if type(C_MountJournal.GetMountIDs) ~= 'function' then return end
        local ok, ids = pcall(C_MountJournal.GetMountIDs)
        if not ok or type(public(ids)) ~= 'table' then return end
        fallback = {}
        for _, id in ipairs(ids) do
            id = Info.Number(id)
            if id then
                local success, name, spell = pcall(C_MountJournal.GetMountInfoByID, id)
                spell = Info.Number(spell)
                if success and spell then fallback[spell] = id end
            end
        end
        if #ids == 0 then fallback = nil; return end
    end
    return fallback[spellID]
end
local function displayedUnit(tooltip)
    if TooltipUtil and type(TooltipUtil.GetDisplayedUnit) == 'function' then
        local ok, name, unit = pcall(TooltipUtil.GetDisplayedUnit, tooltip)
        unit = public(unit)
        if ok and type(unit) == 'string' then return unit end
    end
    if type(tooltip.GetUnit) == 'function' then
        local ok, name, unit = pcall(tooltip.GetUnit, tooltip)
        unit = public(unit)
        if ok and type(unit) == 'string' then return unit end
    end
end
local function auraSpell(unit, index)
    if C_UnitAuras and type(C_UnitAuras.GetAuraDataByIndex) == 'function' then
        local ok, aura = pcall(C_UnitAuras.GetAuraDataByIndex, unit, index, 'HELPFUL')
        aura = public(aura)
        if not ok or type(aura) ~= 'table' then return nil, true end
        return Info.Number(aura.spellId), false
    elseif type(UnitBuff) == 'function' then
        local ok, name, icon, count, kind, duration, expiry, caster, stealable, personal, spell, legacySpell = pcall(UnitBuff, unit, index)
        name = public(name)
        if not ok or name == nil then return nil, true end
        return Info.Number(spell) or Info.Number(legacySpell), false
    end
    return nil, true
end
local function augment(tooltip)
    if not Info.db or not Info.db.mounts or tooltip ~= GameTooltip or adding or marked or blocked() or not journal() then return end
    if tooltip.IsForbidden and tooltip:IsForbidden() then return end
    local unit = displayedUnit(tooltip)
    if not unit or type(UnitIsPlayer) ~= 'function' then return end
    local ok, player = pcall(UnitIsPlayer, unit)
    if not ok or public(player) ~= true then return end
    for index = 1, 255 do
        local spell, finished = auraSpell(unit, index)
        if finished then return end
        local id = spell and mountForSpell(spell)
        if id then
            local success, name, mountSpell, icon, active, usable, sourceType, favorite, factionSpecific, faction, hidden, collected = pcall(C_MountJournal.GetMountInfoByID, id)
            name, icon, collected = public(name), public(icon), public(collected)
            if not success or type(name) ~= 'string' then return end
            local source
            if Info.db.source and type(C_MountJournal.GetMountInfoExtraByID) == 'function' then
                local sourceOK, display, description, text = pcall(C_MountJournal.GetMountInfoExtraByID, id)
                text = public(text)
                if sourceOK and type(text) == 'string' and text:match('%S') then source = text end
            end
            adding, marked = true, true
            tooltip:AddLine(' ')
            local title = 'Mount: ' .. name
            if type(icon) == 'number' then title = '|T' .. icon .. ':16:16|t ' .. title end
            local owned = collected == true and (COLLECTED or 'Collected') or collected == false and (NOT_COLLECTED or 'Not collected') or ''
            tooltip:AddDoubleLine(title, owned, 0.61, 0.71, 1, 0.75, 0.77, 0.8)
            if Info.db.source then tooltip:AddLine(source or 'Source not listed in the Mount Journal.', 0.75, 0.77, 0.8, true) end
            tooltip:Show()
            adding = false
            return
        end
    end
end

function Info.InstallMountTooltip()
    if installed or not GameTooltip then return end
    local function cleared() marked = false end
    GameTooltip:HookScript('OnTooltipCleared', cleared)
    GameTooltip:HookScript('OnHide', cleared)
    if TooltipDataProcessor and TooltipDataProcessor.AddTooltipPostCall and Enum and Enum.TooltipDataType then
        TooltipDataProcessor.AddTooltipPostCall(Enum.TooltipDataType.Unit, augment)
        installed = true
    elseif not GameTooltip.HasScript or GameTooltip:HasScript('OnTooltipSetUnit') then
        local ok = pcall(GameTooltip.HookScript, GameTooltip, 'OnTooltipSetUnit', augment)
        installed = ok
    end
end
