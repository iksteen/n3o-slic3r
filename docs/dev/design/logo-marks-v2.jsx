// logo-marks-v2.jsx — Tauri-inspired orbital marks for n3o-slic3r.
// The Tauri logo language: a bold open ring (orbit) + a floating dot.
// Here the orbiting dots/nozzles ARE the toolheads — multiple bodies on one
// orbit tells the multi-toolhead story; the accent dot is the 2nd material.
// All monochrome via currentColor; pass `grad` for a Tauri-style sweep fill.

const { useId } = React;

// 0deg = top, increasing clockwise.
function polar(cx, cy, r, deg) {
  const a = (deg - 90) * Math.PI / 180;
  return [cx + r * Math.cos(a), cy + r * Math.sin(a)];
}
function arc(cx, cy, r, a0, a1) {
  const [x0, y0] = polar(cx, cy, r, a0);
  const [x1, y1] = polar(cx, cy, r, a1);
  const large = ((a1 - a0) % 360 + 360) % 360 > 180 ? 1 : 0;
  return `M${x0.toFixed(2)} ${y0.toFixed(2)} A${r} ${r} 0 ${large} 1 ${x1.toFixed(2)} ${y1.toFixed(2)}`;
}

function Grad({ id }) {
  return (
    <linearGradient id={id} x1="8" y1="41" x2="40" y2="7" gradientUnits="userSpaceOnUse">
      <stop offset="0" stopColor="#FFC42E"/>
      <stop offset="1" stopColor="#1ECFE0"/>
    </linearGradient>
  );
}

// ── A · Orbit Duo — two toolhead bodies riding one orbit, gap upper-right.
function MarkOrbitDuo({ accent, grad }) {
  const id = useId();
  const stroke = grad ? `url(#${id})` : "currentColor";
  return (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      {grad && <defs><Grad id={id}/></defs>}
      <path d={arc(24, 24, 15, 35, 300)} stroke={stroke} strokeWidth="5" strokeLinecap="round"/>
      <circle cx={polar(24,24,15,300)[0]} cy={polar(24,24,15,300)[1]} r="5" fill={stroke}/>
      <circle cx={polar(24,24,15,35)[0]}  cy={polar(24,24,15,35)[1]}  r="5" fill={accent || stroke}/>
      <circle cx="24" cy="24" r="2.4" fill="currentColor" opacity="0.55"/>
    </svg>
  );
}

// ── B · Orbit Nozzle — orbit terminates in a nozzle aimed at the print dot.
function MarkOrbitNozzle({ accent, grad }) {
  const id = useId();
  const stroke = grad ? `url(#${id})` : "currentColor";
  const [hx, hy] = polar(24, 24, 15, 300); // nozzle anchor on the orbit
  return (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      {grad && <defs><Grad id={id}/></defs>}
      <path d={arc(24, 24, 15, 40, 300)} stroke={stroke} strokeWidth="5" strokeLinecap="round"/>
      {/* second toolhead, a floating body */}
      <circle cx={polar(24,24,15,40)[0]} cy={polar(24,24,15,40)[1]} r="5" fill={accent || stroke}/>
      {/* nozzle: small wedge from the orbit pointing inward */}
      <path d={`M${hx-3.4} ${hy-2.2} L${hx+1.8} ${hy-3.6} L24 24 Z`} fill={stroke}/>
      <circle cx="24" cy="24" r="3" fill="currentColor"/>
    </svg>
  );
}

// ── C · Tri-Orbit — three toolheads spaced around the orbit.
function MarkTriOrbit({ accent, grad }) {
  const id = useId();
  const stroke = grad ? `url(#${id})` : "currentColor";
  const angles = [0, 120, 240];
  return (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      {grad && <defs><Grad id={id}/></defs>}
      <circle cx="24" cy="24" r="15" stroke={stroke} strokeWidth="3.6"/>
      {angles.map((a, i) => {
        const [x, y] = polar(24, 24, 15, a);
        return <circle key={i} cx={x} cy={y} r="5" fill={i === 1 ? (accent || stroke) : stroke}/>;
      })}
      <circle cx="24" cy="24" r="2.4" fill="currentColor" opacity="0.55"/>
    </svg>
  );
}

// ── D · Comet — Tauri's tapering swoosh; fat head + trailing toolhead dot.
function MarkComet({ accent, grad }) {
  const id = useId();
  const stroke = grad ? `url(#${id})` : "currentColor";
  const [hx, hy] = polar(24, 24, 15, 300);
  const [tx, ty] = polar(24, 24, 15, 70);
  return (
    <svg viewBox="0 0 48 48" fill="none" aria-hidden="true">
      {grad && <defs><Grad id={id}/></defs>}
      <path d={arc(24, 24, 15, 70, 300)} stroke={stroke} strokeWidth="5" strokeLinecap="round"/>
      <circle cx={hx} cy={hy} r="6.5" fill={stroke}/>
      <circle cx={tx} cy={ty} r="3.4" fill={accent || stroke}/>
    </svg>
  );
}

window.LogoMarksV2 = { MarkOrbitDuo, MarkOrbitNozzle, MarkTriOrbit, MarkComet };
