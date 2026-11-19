import { useEffect, useRef, useState, type ReactNode } from 'react';
import { TableMenu } from './plot/FigureMenu';
import { useStore } from './store';
import { num, toMeters, unitLabel, type Units } from './units';

/**
 * The button that starts a long job of `kind`; while that job runs it becomes its Stop
 * button, showing the stage and elapsed time.
 */
export function JobButton(props: { kind: string; label: ReactNode; onClick: () => void; disabled?: boolean; title?: string }) {
  const job = useStore((s) => s.app?.job);
  const stop = useStore((s) => s.stop);
  if (job?.state === 'running' && job.kind === props.kind) {
    return (
      <button className="danger" disabled={job.stopping} onClick={() => stop()} title="Stop at the next checkpoint (meshing finishes its current pass first)">
        {job.stopping ? 'Stopping…' : `■ Stop`}
        <span className="job-time">{(job.elapsed ?? 0).toFixed(0)} s</span>
      </button>
    );
  }
  return (
    <button className="primary" disabled={props.disabled || job?.state === 'running'} onClick={props.onClick} title={props.title}>
      {props.label}
    </button>
  );
}

/** Error or "stopped" note of the last job of `kind`. */
export function JobOutcome({ kind }: { kind: string }) {
  const job = useStore((s) => s.app?.job);
  if (job?.kind !== kind) return null;
  if (job.state === 'failed') return <div className="error">{job.error}</div>;
  if (job.state === 'stopped') return <div className="hint">Stopped.</div>;
  return null;
}

/** Numeric input showing a length (stored in meters) in display units. */
export function LengthField(props: { value: number; units: Units; onChange: (m: number) => void; label?: string; title?: string }) {
  return (
    <NumberField
      label={props.label}
      title={props.title}
      value={Number(num(props.value, props.units, 6))}
      suffix={unitLabel(props.units)}
      onChange={(v) => props.onChange(toMeters(v, props.units))}
    />
  );
}

/** Commit-on-blur/Enter numeric input. */
export function NumberField(props: { value: number; onChange: (v: number) => void; label?: string; suffix?: string; title?: string; step?: number }) {
  const [text, setText] = useState(String(props.value));
  useEffect(() => setText(String(props.value)), [props.value]);
  const commit = () => {
    const v = Number(text);
    if (Number.isFinite(v) && text.trim() !== '') {
      if (v !== props.value) props.onChange(v);
    } else setText(String(props.value));
  };
  return (
    <label className="field" title={props.title}>
      {props.label && <span className="field-label">{props.label}</span>}
      <input
        type="text"
        inputMode="decimal"
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
        }}
      />
      {props.suffix && <span className="field-suffix">{props.suffix}</span>}
    </label>
  );
}

export function Section(props: { title: string; children: ReactNode; right?: ReactNode; defaultOpen?: boolean }) {
  const [open, setOpen] = useState(props.defaultOpen ?? true);
  return (
    <section className="section">
      <header onClick={() => setOpen(!open)}>
        <span className={`chev ${open ? 'open' : ''}`}>▸</span>
        <h3>{props.title}</h3>
        <span className="section-right" onClick={(e) => e.stopPropagation()}>
          {props.right}
        </span>
      </header>
      {open && <div className="section-body">{props.children}</div>}
    </section>
  );
}

/** A figure or table in the results area. */
export function Card(props: { title: ReactNode; children: ReactNode; right?: ReactNode; wide?: boolean; className?: string; table?: boolean }) {
  const body = useRef<HTMLDivElement>(null);
  return (
    <section className={`card ${props.wide ? 'wide' : ''} ${props.className ?? ''}`}>
      <header>
        <h3>{props.title}</h3>
        {(props.right || props.table) && (
          <span className="card-right">
            {props.right}
            {props.table && <TableMenu root={() => body.current} name={typeof props.title === 'string' ? props.title : 'table'} />}
          </span>
        )}
      </header>
      <div className="card-body" ref={body}>
        {props.children}
      </div>
    </section>
  );
}

export function KV(props: { k: ReactNode; v: ReactNode; title?: string }) {
  return (
    <div className="kv" title={props.title}>
      <span className="k">{props.k}</span>
      <span className="v">{props.v}</span>
    </div>
  );
}

export function Seg<T extends string>(props: { value: T; options: { value: T; label: ReactNode; title?: string }[]; onChange: (v: T) => void }) {
  return (
    <div className="seg">
      {props.options.map((o) => (
        <button key={o.value} className={o.value === props.value ? 'on' : ''} title={o.title} onClick={() => props.onChange(o.value)}>
          {o.label}
        </button>
      ))}
    </div>
  );
}
