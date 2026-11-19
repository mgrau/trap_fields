// Runs one benchmark program (gmsh mesher or tf-bench solver) inside a Web Worker.
// Messages in: { prog: 'mesh' | 'solve' }. Messages out: { line } per output line, { done, seconds }.
self.onmessage = async ({ data }) => {
  const t0 = performance.now();
  const line = (s) => self.postMessage({ line: s });
  const files = async (m, names) => {
    for (const n of names) m.FS.writeFile('/' + n, new Uint8Array(await (await fetch(n)).arrayBuffer()));
  };
  try {
    if (data.prog === 'mesh') {
      importScripts('mesh_step.js');
      const t = performance.now();
      const m = await GmshModule({ print: line, printErr: (s) => line('[err] ' + s) });
      line(`module load ${((performance.now() - t) / 1000).toFixed(2)} s`);
      await files(m, ['trap_electrodes_nothread.step']);
      // the 15.6k-panel benchmark sizes: GUI defaults x 1.4
      m.callMain(['/trap_electrodes_nothread.step', '4.2e-5', '4.06e-3']);
    } else {
      importScripts('tf-bench.js');
      const t = performance.now();
      const m = await TfBench({ print: line, printErr: (s) => line('[err] ' + s) });
      line(`module load ${((performance.now() - t) / 1000).toFixed(2)} s`);
      await files(m, ['mesh_15594.bin', 'mesh_15594.nodes.f64', 'trap_electrodes_nothread.step']);
      m.callMain(['/mesh_15594.bin', '--nodes', '/mesh_15594.nodes.f64', '--step', '/trap_electrodes_nothread.step']);
    }
  } catch (e) {
    line('exception: ' + e);
  }
  self.postMessage({ done: true, seconds: (performance.now() - t0) / 1000 });
};
