local source = (ASTRAL_SOURCE_ROOT or '.') .. '/addons/AstralForeverStyle/Style.lua'
local frame, event, appliedScale, appliedBars
local function setup(interface)
    GetBuildInfo = function() return '', '', '', interface end
    local live = { bars = { MainBar = { enabled = false, custom = 'kept' } } }
    EllesmereUIDB = { activeProfile = 'Existing', profiles = { Existing = { addons = { EllesmereUIActionBars = live } } } }
    CreateFrame = function()
        frame = {RegisterEvent=function() end,UnregisterAllEvents=function() end,SetScript=function(_,_,fn) event=fn end}
        return frame
    end
    C_Timer = {After=function(_,fn) fn() end}
    InCombatLockdown = function() return false end
    EllesmereUI = { PP = { SetUIScale = function(scale) appliedScale=scale end } }
    _EAB_Apply = function() appliedBars = true end
    appliedScale, appliedBars = nil, nil
    return live
end
local live = setup(16001)
dofile(source)
assert(EllesmereUIDB.profiles.Existing.addons.EllesmereUIActionBars == live)
assert(live.bars.MainBar.custom == 'kept')
assert(live.bars.MainBar.enabled and live.bars.MainBar.buttonWidth == 30)
assert(live.bars.StanceBar.buttonWidth == 24 and live.bars.StanceBar.overrideNumIcons == nil)
assert(live.barPositions.MainBar.point == 'BOTTOM' and live.barPositions.MainBar.y == 31)
assert(live.barPositions.Bar2.y == 133 and live.barPositions.Bar9.point == 'BOTTOMRIGHT')
assert(not live.bars.Bar6.enabled and not live.bars.Bar10.enabled)
live.bars.MainBar.buttonWidth = 42
assert(live.bars.Bar2.buttonWidth == 30)
event(frame, 'PLAYER_LOGIN')
assert(appliedScale == 0.64 and appliedBars)
dofile(source)
assert(live.bars.MainBar.buttonWidth == 42)
assert(EllesmereUIDB.activeProfile == 'Existing')
live = setup(120105)
dofile(source)
assert(EllesmereUIDB._astralForeverStyle == nil)
assert(live.bars.MainBar.buttonWidth == nil)
live = setup(16001)
EllesmereUIDB.firstInstallPopupShown = true
dofile(source)
assert(EllesmereUIDB._astralForeverStyle == nil)
assert(live.bars.MainBar.buttonWidth == nil)
print('Forever style seed and live profile tests passed')
