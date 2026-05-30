-- filament-summary: a read-only demonstration of the `filament`
-- binding (the third argument to the slice hooks).
--
-- It prepends a comment block to the G-code listing each bound slot's
-- material and identity — purely informational, it never changes the
-- toolpath. Use it as a template for material-aware plugins.
--
-- The `filament` table is the slice-time material→slot mapping (what
-- each slot is *bound* to), not a live driver readout: there's no
-- "loaded" flag and no mismatch state here.

local function describe(slot)
  if not slot.bound then
    return "slot " .. slot.index .. " (" .. slot.feed .. "): <empty>"
  end
  local parts = { slot.type or "?" }
  if slot.color then parts[#parts + 1] = slot.color end
  if slot.vendor then parts[#parts + 1] = slot.vendor end
  return "slot " .. slot.index .. " (" .. slot.feed .. "): " ..
         slot.identity .. " [" .. table.concat(parts, " ") .. "]"
end

function on_post_slice(gcode, plate, filament)
  local p = filament:printer()
  -- Insert at the very top so the header is the first thing in the file.
  -- Walk backwards so each insert(1, ...) leaves lines in source order.
  local lines = {}
  lines[#lines + 1] = "; n3o filament loadout for " .. p.model
  for _, slot in ipairs(filament:slots()) do
    lines[#lines + 1] = "; " .. describe(slot)
  end
  if filament:count() == 0 then
    lines[#lines + 1] = "; (no printer instance resolved — empty loadout)"
  end
  for i = #lines, 1, -1 do
    gcode:insert(1, lines[i])
  end
end
