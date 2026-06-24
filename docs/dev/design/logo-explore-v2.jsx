// logo-explore-v2.jsx — Tauri-inspired orbital marks, in context.
const { MarkOrbitDuo, MarkOrbitNozzle, MarkTriOrbit, MarkComet } = window.LogoMarksV2;

function TopbarStrip({ theme, Mark }) {
  return (
    <div className="ls-strip" data-theme={theme}>
      <div className="ls-brand">
        <span className="ls-mark ls-mark-22"><Mark grad/></span>
        <span className="ls-word">n3o-slic3r</span>
        <svg className="ls-caret" width="9" height="9" viewBox="0 0 10 10" fill="none">
          <path d="M2 4l3 3 3-3" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round"/>
        </svg>
      </div>
      <div className="ls-strip-div"/>
      <div className="ls-file">
        <svg width="12" height="12" viewBox="0 0 14 14" fill="none">
          <path d="M2 3.5A1.5 1.5 0 0 1 3.5 2h2.4l1.4 1.5H10.5A1.5 1.5 0 0 1 12 5v5.5A1.5 1.5 0 0 1 10.5 12h-7A1.5 1.5 0 0 1 2 10.5v-7z" stroke="currentColor" strokeWidth="1.2" strokeLinejoin="round"/>
        </svg>
        balcony-clip
      </div>
    </div>
  );
}

function Card({ Mark, note }) {
  return (
    <div className="lc">
      <div className="lc-hero">
        <span className="ls-mark lc-hero-mark"><Mark grad/></span>
        <span className="ls-mark lc-hero-mono"><Mark accent="var(--accent)"/></span>
        <span className="ls-mark lc-hero-tiny"><Mark/></span>
      </div>
      <TopbarStrip theme="light" Mark={Mark}/>
      <TopbarStrip theme="dark" Mark={Mark}/>
      <p className="lc-note">{note}</p>
    </div>
  );
}

function App() {
  return (
    <DesignCanvas>
      <DCSection id="orbit" title="Orbital marks" subtitle="Tauri-inspired: a bold orbit where the bodies riding it are the toolheads">
        <DCArtboard id="duo" label="A · Orbit Duo" width={320} height={420}>
          <Card Mark={MarkOrbitDuo} note="Two toolhead bodies riding one open orbit — the most direct read of Tauri's ring-and-dot as a multi-toolhead system."/>
        </DCArtboard>
        <DCArtboard id="nozzle" label="B · Orbit Nozzle" width={320} height={420}>
          <Card Mark={MarkOrbitNozzle} note="The orbit terminates in a nozzle aimed at the print dot at center; a second body trails it. Adds the literal 'extrusion' beat."/>
        </DCArtboard>
        <DCArtboard id="tri" label="C · Tri-Orbit" width={320} height={420}>
          <Card Mark={MarkTriOrbit} note="Three toolheads evenly spaced on the orbit. Calmest and most symmetric; says 'many heads' at a glance."/>
        </DCArtboard>
        <DCArtboard id="comet" label="D · Comet" width={320} height={420}>
          <Card Mark={MarkComet} note="Tauri's tapering swoosh: a fat leading head and a trailing second toolhead. Most energetic / motion-forward."/>
        </DCArtboard>
      </DCSection>
    </DesignCanvas>
  );
}

ReactDOM.createRoot(document.getElementById("app")).render(<App/>);
