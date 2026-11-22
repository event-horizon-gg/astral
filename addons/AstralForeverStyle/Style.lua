-- Only initializes a fresh Forever installation. Later manual changes are kept.
local _, _, _, interface = GetBuildInfo()
if interface < 16000 or interface >= 16100 then return end
local db = EllesmereUIDB
if not db or db._astralForeverStyle or db.firstInstallPopupShown then return end

local function bar(size, icons)
    return {
        enabled = true, buttonWidth = size, buttonHeight = size, buttonPadding = 4,
        overrideNumIcons = icons, overrideNumRows = 1, alwaysShowButtons = true,
        keybindFontSize = 11, countFontSize = 11, macroFontSize = 11,
    }
end

local function anchor(point, x, y)
    return { point = point, relPoint = point, x = x, y = y }
end

local seed = {
    activeProfile = "Astral Forever",
    profiles = {
        ["Astral Forever"] = { addons = { EllesmereUIActionBars = {
            bars = {
                MainBar = bar(30, 12), Bar3 = bar(30, 12), Bar4 = bar(30, 12),
                Bar2 = bar(30, 12), Bar9 = bar(30, 12), Bar5 = bar(30, 12),
                StanceBar = bar(24), PetBar = bar(24),
                Bar6 = { enabled = false }, Bar7 = { enabled = false },
                Bar8 = { enabled = false }, Bar10 = { enabled = false },
            },
            barPositions = {
                MainBar = anchor("BOTTOM", 0, 31), Bar3 = anchor("BOTTOM", 0, 65),
                Bar4 = anchor("BOTTOM", 0, 99), Bar2 = anchor("BOTTOM", 0, 133),
                Bar9 = anchor("BOTTOMRIGHT", -4, 176), Bar5 = anchor("BOTTOMRIGHT", -4, 212),
                StanceBar = anchor("BOTTOMLEFT", 447, 15), PetBar = anchor("BOTTOM", -302, 176),
            },
            useBlizzardStyle = false, useClassicStyle = false, useBlizzardDataBars = false,
        } } },
    },
    ppUIScale = 0.64, ppUIScaleAuto = false, _capturedOnce_EAB = true,
    firstInstallPopupShown = true, bagsUserChosen = true,
}

-- Ellesmere's live db.profile pointers must keep referring to the same tables
-- regardless of the order in which child modules loaded.
local active = db.activeProfile or (db.profiles and db.profiles.Default and "Default") or "Astral Forever"
if active ~= "Astral Forever" then
    seed.profiles[active] = seed.profiles["Astral Forever"]
    seed.profiles["Astral Forever"] = nil
    seed.activeProfile = active
end
local function merge(dst, src)
    for key, value in pairs(src) do
        if type(value) == "table" then
            dst[key] = type(dst[key]) == "table" and dst[key] or {}
            merge(dst[key], value)
        else dst[key] = value end
    end
end
merge(db, seed)
db._astralForeverStyle = 1
local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN")
frame:SetScript("OnEvent", function(self)
    self:UnregisterAllEvents()
    C_Timer.After(2, function()
        if InCombatLockdown() then return end
        if EllesmereUI and EllesmereUI.PP then EllesmereUI.PP.SetUIScale(db.ppUIScale) end
        if _EAB_Apply then _EAB_Apply() end
        print("|cffbce8cfAstral:|r Your Forever interface is ready. Use /eui to adjust it. Retail settings stay separate.")
    end)
end)
