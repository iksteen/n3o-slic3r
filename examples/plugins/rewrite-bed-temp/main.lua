-- rewrite-bed-temp-by-range: clamp the resolved bed temperature into a
-- fixed band before the slice.
--
-- A pre-slice example: `settings` is the resolved cascade as a
-- table-like view (read/write strings), `ctx` is read-only context
-- (printer model, plate, slot count). The band is a constant for now;
-- it becomes a plugin-declared setting once those are wired.
local MIN, MAX = 50, 60

function on_pre_slice(settings, ctx)
  local bed = tonumber(settings.bed_temp)
  if bed == nil then
    return
  end
  if bed < MIN then
    settings.bed_temp = tostring(MIN)
  elseif bed > MAX then
    settings.bed_temp = tostring(MAX)
  end
end
