-- beep-at-layer: insert an M300 beep at the start of a layer.
--
-- A tiny post-slice example: iterate the sliced G-code's layers and, at
-- the chosen layer, insert a beep command right at the boundary.
--
-- The target layer is the plugin's `layer` setting, read off the
-- `settings` global (resolved per slice from the cascade — the manifest
-- default unless overridden at a level where the plugin is on).

function on_post_slice(gcode, plate)
  for layer in gcode:layers() do
    if layer.index == settings.layer then
      gcode:insert(layer.first_line, "M300 S440 P200")
    end
  end
end
