-- pause-at-layer: insert a pause at the start of a layer.
--
-- Useful for embedding magnets/nuts, swapping filament by hand, or
-- inspecting a print mid-way. M0 is the generic "unconditional stop";
-- adjust to your firmware's pause (e.g. M601 on Marlin, a macro on
-- Klipper) by editing the command below.
--
-- The target layer is the plugin's `layer` setting, read off the
-- `settings` global (resolved per slice from the cascade).

function on_post_slice(gcode, plate)
  for layer in gcode:layers() do
    if layer.index == settings.layer then
      gcode:insert(layer.first_line, "M0 ; n3o pause-at-layer")
    end
  end
end
