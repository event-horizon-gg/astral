local root = (ASTRAL_SOURCE_ROOT or '.') .. '/addons/AstralInfo/'
local function setup(legacy, noJournal)
    local frames, calls, info = {}, { fps = 0, net = 0, auras = 0, journal = 0 }, {}
    local secret = setmetatable({}, {__tostring=function() error('secret was formatted') end})
    local combat, auraSecret, unitSecret, sourceMissing = false, false, false, false
    local function noop() end
    UIParent, STANDARD_TEXT_FONT = {}, 'Fonts\\FRIZQT__.TTF'
    AstralInfoDB = nil
    SlashCmdList = {}
    print = noop
    issecretvalue = function(value) return value == secret end
    InCombatLockdown = function() return combat end
    IsLoggedIn = function() return false end
    GetFramerate = function() calls.fps = calls.fps + 1; return 61.6 end
    GetNetStats = function() calls.net = calls.net + 1; return 0, 0, 21, 42 end
    UnitIsPlayer = function(unit) return unit == 'mouseover' end
    CreateFrame = function(kind, name)
        local f = {scripts={}, point={'TOPRIGHT',UIParent,'TOPRIGHT',-24,-220},shown=true}
        for _, method in ipairs({'SetFrameStrata','SetClampedToScreen','SetMovable','EnableMouse','RegisterForDrag','SetSize','ClearAllPoints','RegisterEvent','UnregisterEvent'}) do f[method] = noop end
        f.SetScript = function(self,event,fn) self.scripts[event]=fn end
        f.SetPoint = function(self,...) self.point={...} end
        f.GetPoint = function(self) return table.unpack(self.point) end
        f.SetShown = function(self,value) self.shown=value;if not value and self.scripts.OnHide then self.scripts.OnHide(self) end end
        f.IsMouseOver = function() return false end
        f.StartMoving = function(self) self.moving=true end
        f.StopMovingOrSizing = function(self) self.moving=false end
        f.CreateTexture = function() return {SetAllPoints=noop,SetColorTexture=noop,SetAlpha=noop} end
        f.CreateFontString = function(self)
            local label={SetPoint=noop,SetFont=noop,SetShadowColor=noop,SetShadowOffset=noop,GetStringWidth=function()return 160 end}
            label.SetText=function(_,value)self.text=value end
            return label
        end
        frames[name or 'events']=f
        return f
    end
    local tooltip={hooks={},lines={}}
    tooltip.HookScript=function(self,event,fn)self.hooks[event]=fn end
    tooltip.HasScript=function()return true end
    tooltip.IsForbidden=function()return false end
    tooltip.GetUnit=function()return 'Other player',unitSecret and secret or 'mouseover' end
    tooltip.SetOwner=function(self,owner)self.owner=owner;self.lines={} end
    tooltip.SetText=function(self,text)self.lines={text};if self.hooks.OnTooltipCleared then self.hooks.OnTooltipCleared(self) end end
    tooltip.AddLine=function(self,text)self.lines[#self.lines+1]=text end
    tooltip.AddDoubleLine=function(self,a,b)self.lines[#self.lines+1]=a .. ' / ' .. b end
    tooltip.Show=noop
    tooltip.Hide=function(self)self.hidden=true;if self.hooks.OnHide then self.hooks.OnHide(self) end end
    tooltip.IsOwned=function(self,owner)return self.owner==owner end
    GameTooltip=tooltip
    TooltipUtil=nil
    local post
    if not legacy then
        TooltipDataProcessor={AddTooltipPostCall=function(_,fn)post=fn end}
        Enum={TooltipDataType={Unit=1}}
        C_UnitAuras={GetAuraDataByIndex=function(unit,index)
            calls.auras=calls.auras+1
            if index==1 then return {spellId=auraSecret and secret or 999} end
            if index==2 then return {spellId=123} end
        end}
        UnitBuff=nil
    else
        TooltipDataProcessor,Enum,C_UnitAuras=nil,nil,nil
        UnitBuff=function(unit,index)
            calls.auras=calls.auras+1
            if index==1 then return 'Mount', nil, 1, nil, nil, nil, nil, nil, nil, 123 end
        end
    end
    C_MountJournal=noJournal and nil or {
        GetMountFromSpell=function(spell)assert(type(spell)=='number','secret spell sent to journal');return spell==123 and 7 or nil end,
        GetMountInfoByID=function(id)
            calls.journal=calls.journal+1
            assert(id==7)
            return 'Example Charger',123,456,false,true,1,false,false,nil,false,false
        end,
        GetMountInfoExtraByID=function()return 1,'Description',sourceMissing and '' or 'Drop: Example boss\nExample dungeon' end,
        GetMountIDs=function()return {7} end,
    }
    if noJournal then C_MountJournal = nil end
    for _, file in ipairs({'Settings.lua','Performance.lua','MountTooltip.lua','Info.lua'}) do
        local chunk=assert(loadfile(root .. file));chunk('AstralInfo',info)
    end
    frames.events.scripts.OnEvent(frames.events,'PLAYER_LOGIN')
    local function clear()tooltip.lines={};tooltip.hooks.OnTooltipCleared(tooltip)end
    return {info=info,frames=frames,calls=calls,tooltip=tooltip,
        hover=function()if post then post(tooltip,{})elseif tooltip.hooks.OnTooltipSetUnit then tooltip.hooks.OnTooltipSetUnit(tooltip)end end,
        clear=clear,combat=function(v)combat=v end,secretAura=function(v)auraSecret=v end,
        secretUnit=function(v)unitSecret=v end,missingSource=function(v)sourceMissing=v end,secret=secret}
end

local t=setup(false)
local widget=t.frames.AstralInfoFrame
assert(widget.text:find('62',1,true) and widget.text:find('42',1,true))
local start=t.calls.fps
for i=1,9 do widget.scripts.OnUpdate(widget,0.1) end
assert(t.calls.fps==start,'stats sampled every frame')
widget.scripts.OnUpdate(widget,0.2)
assert(t.calls.fps==start+1)
SlashCmdList.ASTRALINFO('latency both')
assert(widget.text:find('H ',1,true) and widget.text:find('W ',1,true))
widget.scripts.OnEnter(widget)
assert(table.concat(t.tooltip.lines,' '):find('Latency (home)',1,true))
SlashCmdList.ASTRALINFO('hide')
assert(not widget.shown and t.tooltip.hidden,'hiding the widget left its tooltip up')
SlashCmdList.ASTRALINFO('show')
widget.scripts.OnDragStart(widget);assert(not widget.moving)
SlashCmdList.ASTRALINFO('unlock');widget.scripts.OnDragStart(widget);assert(widget.moving)
widget.point={'BOTTOMLEFT',UIParent,'BOTTOMLEFT',30,40};widget.scripts.OnDragStop(widget)
assert(t.info.db.x==30 and t.info.db.y==40)
SlashCmdList.ASTRALINFO('lock');assert(t.info.db.locked)
local invalid=t.info.Configure({size=0/0,x=math.huge,point='invalid',latency='invalid'})
assert(invalid.size==12 and invalid.x==-24 and invalid.point=='TOPRIGHT' and invalid.latency=='world')
GetFramerate=function()return 0/0 end;GetNetStats=function()return 0,0,nil,nil end
widget.scripts.OnUpdate(widget,1)
assert(widget.text:find('--',1,true))

t.clear();t.hover()
local lines=table.concat(t.tooltip.lines,' ')
assert(lines:find('Mount: Example Charger',1,true) and lines:find('Not collected',1,true))
assert(lines:find('Example boss',1,true) and lines:find('Example dungeon',1,true))
local count=#t.tooltip.lines;t.hover();assert(#t.tooltip.lines==count,'mount lines duplicated')
t.clear();t.combat(true);local scans=t.calls.auras;t.hover();assert(t.calls.auras==scans and #t.tooltip.lines==0)
t.combat(false);t.secretUnit(true);t.hover();assert(t.calls.auras==scans)
t.secretUnit(false);t.secretAura(true);t.hover();assert(#t.tooltip.lines>0,'public mount after a secret unrelated aura was missed')
t.clear();UnitIsPlayer=function()return t.secret end;t.hover();assert(#t.tooltip.lines==0)
UnitIsPlayer=function()return true end
TooltipUtil={GetDisplayedUnit=function()return 'Player','mouseover' end}
t.hover();assert(#t.tooltip.lines>0)
t.clear();t.secretAura(false);t.missingSource(true);t.hover();assert(table.concat(t.tooltip.lines,' '):find('Source not listed',1,true))
SlashCmdList.ASTRALINFO('mounts off');t.clear();t.hover();assert(#t.tooltip.lines==0)

t=setup(true);t.hover();assert(table.concat(t.tooltip.lines,' '):find('Example Charger',1,true))
t=setup(true);UnitBuff=function(_,index)if index==1 then return table.unpack({[1]='Mount',[3]=456,[11]=123},1,11)end end
t.hover();assert(table.concat(t.tooltip.lines,' '):find('Example Charger',1,true))
t=setup(true);C_MountJournal.GetMountFromSpell=nil;t.hover();assert(table.concat(t.tooltip.lines,' '):find('Example Charger',1,true))
t=setup(false,true);t.hover();assert(#t.tooltip.lines==0 and t.frames.AstralInfoFrame.text)
