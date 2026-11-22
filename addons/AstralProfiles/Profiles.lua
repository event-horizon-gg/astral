local handheld = "Astral Handheld"
local desktopBars = { "MainBar", "Bar2", "Bar3", "Bar4" }

local function isForever()
    local _, _, _, interface = GetBuildInfo()
    return interface and interface >= 16000 and interface < 16100
end

local function controllerBarsAvailable()
    return isForever() or (C_AddOns and C_AddOns.IsAddOnLoaded("ConsolePort_Bar"))
end

local function hideDesktopBars(profile)
    if not controllerBarsAvailable() then return end
    profile.addons = profile.addons or {}
    local barsModule = profile.addons.EllesmereUIActionBars or {}
    profile.addons.EllesmereUIActionBars = barsModule
    barsModule.bars = barsModule.bars or {}
    for _, key in ipairs(desktopBars) do
        barsModule.bars[key] = barsModule.bars[key] or {}
        barsModule.bars[key].enabled = false
    end
end

local function message(text)
    print("Astral Profiles: " .. text)
end

local function profileAPI()
    local eui = EllesmereUI
    if eui and eui.GetActiveProfileName and eui.GetProfileList
        and eui.SaveCurrentAsProfile and eui.SwitchProfile then return eui end
end

local function detachModule(folder, target)
    local modules = EllesmereUIDB and EllesmereUIDB.syncedModules
    local shared = modules and modules[folder]
    if type(shared) == "table" then shared[target] = nil end
end

local function switchLayout(eui, db, target, active, repairBars)
    -- Ellesmere's shared module handoff would propagate Handheld visibility to
    -- Regular unless it is detached before switching.
    if repairBars then detachModule("EllesmereUIActionBars", target) end
    local _, profiles = eui.GetProfileList()
    if profiles[target] then
        eui.SwitchProfile(target)
    elseif target == handheld then
        eui.SaveCurrentAsProfile(target)
        -- Ellesmere links bags by default when it copies a profile.
        detachModule("EllesmereUIBags", target)
    else
        return false, "The saved desktop layout is no longer available."
    end
    if eui.GetActiveProfileName() ~= target then
        return false, "Ellesmere couldn't switch layouts."
    end
    if repairBars then
        local _, updated = eui.GetProfileList()
        hideDesktopBars(updated[target])
        -- Effective spec/group layers can have separate live profile tables.
        local registry = eui.Lite and eui.Lite._dbRegistry or {}
        for _, live in ipairs(registry) do
            if live.folder == "EllesmereUIActionBars" and live.profile then
                hideDesktopBars({addons={EllesmereUIActionBars=live.profile}})
            end
        end
    end
    if target == handheld and active ~= target then db.desktopProfile = active end
    return true
end

SLASH_ASTRALPROFILES1 = "/astralprofile"
SlashCmdList.ASTRALPROFILES = function(input)
    local mode = (input or ""):lower():match("^%s*(.-)%s*$")
    if mode ~= "handheld" and mode ~= "desktop" then
        message("/astralprofile handheld or /astralprofile desktop")
        return
    end
    local eui = profileAPI()
    if not (eui and eui.ShowConfirmPopup) then
        message("Update EllesmereUI, then try again.")
        return
    end
    if InCombatLockdown() then
        message("Leave combat before switching layouts.")
        return
    end
    AstralProfilesDB = AstralProfilesDB or {}
    local db = AstralProfilesDB
    local active = eui.GetActiveProfileName()
    local _, profiles = eui.GetProfileList()
    local target = mode == "handheld" and handheld or db.desktopProfile
    if mode == "desktop" and (not target or not profiles[target]) then
        message("Choose your desktop layout in Ellesmere's Profiles page first.")
        return
    end
    local repairBars = mode == "handheld" and controllerBarsAvailable()
    if active == target and not repairBars then
        message(target .. " is already active.")
        return
    end
    eui:ShowConfirmPopup({
        title = "Switch to " .. target,
        message = "Your UI will reload.",
        confirmText = "Switch and reload",
        cancelText = "Cancel",
        onConfirm = function()
            if InCombatLockdown() or eui.GetActiveProfileName() ~= active then
                message("The current layout changed or combat started. Try again.")
                return
            end
            local ok, error = switchLayout(eui, db, target, active, repairBars)
            if not ok then message(error); return end
            ReloadUI()
        end,
    })
end

-- The desktop profile switch writes this owned SavedVariable. Applying it on
-- login needs no slash command and must not create a second controller HUD.
local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:RegisterEvent("PLAYER_REGEN_ENABLED")
frame:RegisterEvent("PLAYER_SPECIALIZATION_CHANGED")
local function applyManagedProfile()
    if InCombatLockdown() then return end
    if AstralProfileMode ~= "handheld" and AstralProfileMode ~= "regular" then return end
    local eui = profileAPI()
    if not eui then return end
    if AstralProfileMode == "handheld" and not controllerBarsAvailable() then
        message("Enable ConsolePort and its Action Bar module for Retail Handheld.")
        return
    end
    AstralProfilesDB = AstralProfilesDB or {}
    local db = AstralProfilesDB
    local active = eui.GetActiveProfileName()
    local target = AstralProfileMode == "handheld" and handheld or db.desktopProfile
    if AstralProfileMode == "regular" and active ~= handheld then return end
    if not target then return end
    if not switchLayout(eui, db, target, active, AstralProfileMode == "handheld") then return end
    if active ~= target then ReloadUI()
    elseif _EAB_Apply then _EAB_Apply() end
end
frame:SetScript("OnEvent", function(_, event, unit)
    if event == "PLAYER_SPECIALIZATION_CHANGED" and unit ~= "player" then return end
    C_Timer.After(0, applyManagedProfile)
end)
