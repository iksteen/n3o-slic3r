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
}
