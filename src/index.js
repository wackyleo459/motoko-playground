import assets from 'ic:canisters/playground_assets';
import * as Wasi from './wasiPolyfill';

const prog = `
import Time "mo:base/Time";
import Prim "mo:prim";
func fac(n: Nat) : Nat {
  if (n == 0) return 1;
  n * fac(n-1);
};
let x = fac(5);
Prim.debugPrint (debug_show x);
Time.now()
`;

async function retrieve(file) {
  const content = await assets.retrieve(file);
  return new TextDecoder().decode(new Uint8Array(content));
}

async function loadBase(lib) {
  Motoko.loadFile(lib, await retrieve(lib));
}

async function init() {
  const dom = document.createElement('div');
  dom.width = "100%";
  document.body.appendChild(dom);
  dom.innerHTML = 'Loading compiler...';
  
  const js = await retrieve('mo_js.js');
  const script = document.createElement('script');
  script.text = js;
  document.body.appendChild(script);
  dom.innerHTML = '';

  loadBase('Time.mo');
  
  const code = document.createElement('textarea');
  code.rows = 25;
  code.style.width = "45%";
  code.value = prog;
  const output = document.createElement('textarea');
  output.rows = 25;
  output.style.width = "45%";
  const run = document.createElement('input');
  run.type = "button";
  run.value = "Run";
  const compile = document.createElement('input');
  compile.type = "button";
  compile.value = "Compile";
  dom.appendChild(code);
  dom.appendChild(output);
  dom.appendChild(document.createElement('br'));
  dom.appendChild(run);
  dom.appendChild(compile);

  run.addEventListener('click', () => {
    output.value = 'Running...';
    try {
      const tStart = Date.now();
      const out = Motoko.run(code.value);
      const duration = (Date.now() - tStart) / 1000;
      output.value = out.stderr + out.stdout + out.result;
      output.value += `\n(run time: ${duration}s)\n`;
    } catch(err) {
      output.value = 'Exception:\n' + err;
      throw err;
    };
  });

  compile.addEventListener('click', () => {
    output.value = 'Compiling...';
    try {
      const tStart = Date.now();
      const out = Motoko.compileWasm("wasi", code.value);
      const duration = (Date.now() - tStart) / 1000;
      if (out.result.code === null) {
        output.value = JSON.stringify(out.result.diagnostics);
      } else {
        output.value = `(compile time: ${duration}s)\n`;
        const wasiPolyfill = new Wasi.barebonesWASI();
        Wasi.importWasmModule(out.result.code, wasiPolyfill, output);
        output.value += out.stderr + out.stdout;
      }
    } catch(err) {
      output.value = 'Exception:\n' + err;
      throw err;
    };
  });
}

init();
