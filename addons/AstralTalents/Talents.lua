local _, ns = ...
local modes = { "Mythic+", "Raid", "Delves" }
local names = { "Astral M+", "Astral Raid", "Astral Delves" }
local panel
local RenderBuilds
local specs = { {102, "Balance"}, {103, "Feral"}, {104, "Guardian"}, {105, "Restoration"} }

local function ShowImport(code, name)
    if not PlayerSpellsFrame then
        PlayerSpellsFrame_LoadUI()
    end

    local spells = PlayerSpellsFrame
    local dialog = ClassTalentLoadoutImportDialog
    if not (spells and spells.talentTabID and dialog and dialog.ShowDialog) then
        return false
    end
    ShowUIPanel(spells)
    spells:SetTab(spells.talentTabID)
    if not (spells:IsShown() and spells.TalentsFrame:IsShown()) then
        return false
    end

    local importBox = dialog.ImportControl:GetEditBox()
    local nameBox = dialog.NameControl:GetEditBox()
    dialog:ShowDialog()
    importBox:SetText(code)
    nameBox:SetText(name)
    return true
end

local function MakePanel()
    local frame = CreateFrame("Frame", "AstralTalentsPanel", UIParent, "BackdropTemplate")
    frame:SetSize(410, 275)
    frame:SetPoint("CENTER")
    frame:SetFrameStrata("DIALOG")
    frame:SetClampedToScreen(true)
    frame:SetBackdrop({ bgFile = "Interface\\Buttons\\WHITE8X8" })
    frame:SetBackdropColor(0.07, 0.08, 0.11, 0.98)
    tinsert(UISpecialFrames, frame:GetName())
    frame:Hide()

    local close = CreateFrame("Button", nil, frame, "UIPanelCloseButton")
    close:SetPoint("TOPRIGHT", -4, -4)

    local title = frame:CreateFontString(nil, "ARTWORK", "GameFontNormalLarge")
    title:SetPoint("TOPLEFT", 20, -18)
    title:SetTextColor(0.65, 0.74, 1)
    title:SetText("Astral Talents")

    local detail = frame:CreateFontString(nil, "ARTWORK", "GameFontHighlightSmall")
    detail:SetPoint("TOPLEFT", title, "BOTTOMLEFT", 0, -8)
    frame.detail = detail

    local status = frame:CreateFontString(nil, "ARTWORK", "GameFontHighlightSmall")
    status:SetPoint("BOTTOMLEFT", 20, 13)
    status:SetWidth(350)
    status:SetJustifyH("LEFT")
    frame.status = status

    frame.specButtons = {}
    for index, spec in ipairs(specs) do
        local button = CreateFrame("Button", nil, frame, "UIPanelButtonTemplate")
        button:SetSize(90, 24)
        button:SetPoint("TOPLEFT", 20 + (index - 1) * 94, -66)
        button:SetText(spec[2])
        button:SetScript("OnClick", function() RenderBuilds(frame, spec[1]) end)
        frame.specButtons[spec[1]] = button
    end

    frame.rows = {}
    for i, mode in ipairs(modes) do
        local label = frame:CreateFontString(nil, "ARTWORK", "GameFontNormal")
        label:SetPoint("TOPLEFT", 20, -104 - (i - 1) * 38)
        label:SetText(mode)

        local hero = frame:CreateFontString(nil, "ARTWORK", "GameFontHighlightSmall")
        hero:SetPoint("TOPLEFT", label, "BOTTOMLEFT", 0, -3)

        local button = CreateFrame("Button", nil, frame, "UIPanelButtonTemplate")
        button:SetSize(86, 24)
        button:SetPoint("LEFT", label, "LEFT", 264, -5)
        button:SetText("Import")
        button:SetScript("OnClick", function(self)
            if InCombatLockdown() or PlayerUtil.GetCurrentSpecID() ~= frame.specID then
                status:SetText("Open Astral Talents again after combat or a spec change.")
                return
            end
            frame:Hide()
            local ok, shown = pcall(ShowImport, self.code, names[i])
            if not (ok and shown) then
                frame:Show()
                status:SetText("Open your Talents tab, then try Import again.")
            end
        end)

        frame.rows[i] = { hero = hero, button = button }
    end

    frame:RegisterEvent("PLAYER_REGEN_DISABLED")
    frame:RegisterEvent("PLAYER_SPECIALIZATION_CHANGED")
    frame:SetScript("OnEvent", function(self, event, unit)
        if event == "PLAYER_REGEN_DISABLED" then
            self:Hide()
        elseif unit == "player" and self:IsShown() then
            RenderBuilds(self, PlayerUtil.GetCurrentSpecID())
        end
    end)
    return frame
end

RenderBuilds = function(panel, specID)
    local _, _, _, interface = GetBuildInfo()
    local active = PlayerUtil.GetCurrentSpecID()
    local builds = interface == ns.interface and ns.builds[specID]
    panel.specID = specID
    panel.detail:SetText("Wowhead  /  Checked " .. ns.checked)
    for id, button in pairs(panel.specButtons) do
        button:SetEnabled(id ~= specID)
    end
    for i, row in ipairs(panel.rows) do
        local build = builds and builds[i]
        row.hero:SetText(build and build[1] or "")
        row.button.code = build and build[2]
        row.button:SetEnabled(build ~= nil and build ~= false and specID == active)
    end
    if interface ~= ns.interface then
        panel.status:SetText("Waiting for builds that match this WoW patch.")
    elseif not builds then
        panel.status:SetText("Switch to a Druid specialization to see builds.")
    elseif specID ~= active then
        panel.status:SetText("Activate this specialization in WoW to import a build.")
    else
        panel.status:SetText("")
    end
end

local function TogglePanel()
    if panel and panel:IsShown() then
        panel:Hide()
        return
    end
    if InCombatLockdown() then
        print("Astral Talents is available after combat.")
        return
    end
    if not panel then panel = MakePanel() end
    RenderBuilds(panel, PlayerUtil.GetCurrentSpecID())
    panel:Show()
end

SLASH_ASTRALTALENTS1 = "/astraltalents"
SlashCmdList.ASTRALTALENTS = TogglePanel

local _, class = UnitClass("player")
if class == "DRUID" then
    EventUtil.ContinueOnAddOnLoaded("Blizzard_PlayerSpells", function()
        local talents = PlayerSpellsFrame and PlayerSpellsFrame.TalentsFrame
        local dropdown = talents and talents.LoadSystem and talents.LoadSystem.Dropdown
        if not dropdown then return end

        local shortcut = CreateFrame("Button", "AstralTalentsShortcut", dropdown, "UIPanelButtonTemplate")
        shortcut:SetSize(114, 24)
        -- Raider.IO uses the left end of this row.
        shortcut:SetPoint("BOTTOMLEFT", dropdown, "TOPLEFT", 204, 16)
        shortcut:SetText("Astral Builds")
        shortcut:SetScript("OnClick", TogglePanel)
    end)
end
