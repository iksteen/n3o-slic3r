// Frontend mirror of `core::profile_library::FilamentFragmentSummary`.
// Lives in its own file so the slot chip strip and the filament
// picker modal can both import it without a cycle (the chip strip
// renders the modal; the modal needs the type).

export interface FilamentSummary {
  identity: string;
  display_name: string;
  base_type: string;
  vendor: string;
  nozzle_temp: number;
  bed_temp: number;
  /** Vendor SKU (e.g. "GFA00" for Bambu PLA Basic). Driver sync
   *  uses this to translate AMS reports back into our
   *  bundled identity. `null` for fragments that don't have one. */
  filament_id: string | null;
  /** True when the user has edited this filament in place (an override
   *  profile exists for its slug). The picker shows a Revert affordance for
   *  edited filaments; every filament is editable regardless. */
  edited?: boolean;
}
