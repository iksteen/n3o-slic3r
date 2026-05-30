-- beep-at-layer: insert an M300 beep at the start of a layer.
--
-- A tiny post-slice example: iterate the sliced G-code's layers and,
-- at the chosen layer, insert a beep command right at the boundary.
--
-- The layer is hardcoded for now; once plugin-declared settings are
-- wired into the cascade, the `layer` setting in plugin.toml drives it.
local LAYER = 1

function on_post_slice(gcode, plate)
  for layer in gcode:layers() do
    if layer.index == LAYER then
      gcode:insert(layer.first_line, "M300 S440 P200")
    end
  end
end
