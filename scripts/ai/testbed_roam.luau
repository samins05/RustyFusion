--[[
    Testbed AI: roams around its spawn point forever and never targets or attacks anyone.
    Used for moving-target rocket data (`!mobmix ... roam`) without mobs swarming the shooter,
    whose attacks interrupt the client's rocket firing.
]]

local b = require("@lib/behaviors")

return function(npc: Npc)
    while true do
        if not b.handle_death(npc) then
            -- taking damage sets a target; drop it so idle_roam keeps wandering
            npc:clear_target()
            b.idle_roam(npc)
        end

        yield()
    end
end
