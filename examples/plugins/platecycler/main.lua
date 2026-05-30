-- platecycler: auto-eject the finished plate and stage a fresh one.
--
-- A post-slice plugin for a Chitu PlateCycler on a Bambu A1 mini. It
-- appends the eject/swap macro to the very tail of the plate's G-code,
-- so when the print finishes the PlateCycler sweeps the part off and a
-- clean plate is ready for the next job.
--
-- The macro is the DEFAULT_SWAP_GCODE from the platecycler tool
-- (github.com/iksteen/platecycler, platecycler.py). It moves the
-- toolhead through a hardware-specific ejection path — VERIFY IT
-- MATCHES YOUR platecycler.py / your machine before running on real
-- hardware; a wrong coordinate can crash the toolhead. It becomes a
-- plugin-declared setting once plugin settings are wired.
--
-- Self-guards on the printer model: `printer_compatibility` in the
-- manifest isn't enforced by the host yet, so the plugin must not
-- append a Chitu macro to some other printer's G-code itself.
--
-- Placement matters: Bambu wraps the runnable G-code in a
-- `; EXECUTABLE_BLOCK_START` / `; EXECUTABLE_BLOCK_END` pair and the
-- firmware ignores anything past END (the trailing config block, etc.).
-- The eject macro is therefore inserted just BEFORE the END marker, so
-- it actually runs — appending at the very tail would leave it outside
-- the executable block and the plate would never eject. (This matches
-- where the platecycler tool's multi-plate concat puts the macro: after
-- a plate's full end-G-code, inside the executable region.)
--
-- Idempotent: a sentinel comment guards against double-inserting if the
-- hook ever runs twice over the same G-code.

local PRINTER = "Bambu Lab A1 mini"
local SENTINEL = "; n3o:platecycler"
local END_MARKER = "EXECUTABLE_BLOCK_END"

local SWAP_GCODE = [[
G0 X-10 F5000
G0 Z175
G0 Y-5 F2000
G0 Y186.5 F2000
G0 Y182 F10000
G0 Z186
G0 X180 F5000
G0 Y120 F500
G0 Y-4 Z175 X-15 F3000
G0 Y145
G0 Y115 F1000
G0 Y25 F500
G0 Y85 F1000
G0 Y180 F1000
G0 X-10 F5000
G4 P500 ; wait
G0 Y186.5 F200
G4 P500 ; wait
G0 Y3 F3000
G0 Y-5 F200
G4 P500 ; wait
G0 Y10 F1000
G0 Z100 Y186 F2000
G0 Y150
G4 P1000 ; wait]]

function on_post_slice(gcode, plate)
  if plate.printer_model ~= PRINTER then
    return
  end

  -- Find the executable-block end marker, scanning back from the tail
  -- (it sits just before the trailing config block).
  local insert_at
  for i = #gcode, 1, -1 do
    local line = gcode:line(i)
    if line.kind == "comment" and line.text and line.text:find(END_MARKER, 1, true) then
      insert_at = i
      break
    end
  end

  -- Already cycled? Our sentinel, if present, sits just before that
  -- marker (or at the tail on the no-marker fallback). Scan a short
  -- window rather than the whole file.
  local from = (insert_at or (#gcode + 1)) - 1
  for i = from, math.max(1, from - 40), -1 do
    local line = gcode:line(i)
    if line.kind == "comment" and line.text == SENTINEL then
      return
    end
  end

  if insert_at then
    -- Insert the sentinel + macro just before END, inside the block.
    gcode:insert(insert_at, SENTINEL)
    gcode:insert(insert_at + 1, SWAP_GCODE)
  else
    -- No executable-block marker (unexpected for the A1 mini) — append.
    gcode:append(SENTINEL)
    gcode:append(SWAP_GCODE)
  end
end
