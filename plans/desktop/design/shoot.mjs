// Screenshots every screen x theme x size and fails on wrapped / clipped / overflowing text.
import { chromium } from 'playwright';
const [file, out] = process.argv.slice(2);
const screens = ['empty','today-fresh','today-degraded','brief','funnel','funnel-nocrm','refresh-running','refresh-done','report','advisor-tool','advisor-stream'];
const sizes = [[1200,800],[900,600]];
const b = await chromium.launch();
let bad = 0;
for (const [w,h] of sizes) for (const theme of ['light','dark']) {
  const p = await b.newPage({ viewport:{width:w,height:h}, deviceScaleFactor:2 });
  for (const s of screens) {
    await p.goto(`file://${file}#${s},${theme},bare`); await p.reload(); await p.waitForTimeout(150);
    const issues = await p.evaluate(() => {
      const out=[], vis=e=>e.offsetParent!==null, W=innerWidth, H=innerHeight;
      const one='th,td,button,label,select,h1,h2,.sub,.cap,.lbl,.why,.line,.day>*,.runhead>*,.strip>div>*,.sum p,.feas dt,.feas dd,.notice li,.notice p,.stat,.sess span,.ws *,.health *,.inl';
      for (const e of document.querySelectorAll('.win '+one)) { if(!vis(e)) continue;
        const r=e.getBoundingClientRect(), lh=parseFloat(getComputedStyle(e).lineHeight)||parseFloat(getComputedStyle(e).fontSize)*1.45;
        const t=(e.className&&typeof e.className==='string'?'.'+e.className:e.tagName)+' "'+e.textContent.trim().slice(0,40)+'"';
        if(r.right>W+0.5||r.left<-0.5) out.push('offscreen '+t);
        if(e.scrollWidth>e.clientWidth+1 && !e.matches('.sess b,.cap')) out.push('clipped '+t);
        if(!e.matches('td,th,.notice li,.inl') && r.height>lh*1.9 && ![...e.children].some(c=>getComputedStyle(c).display!=='inline')) out.push('wrapped '+t);
        if(e.matches('td,th') && r.height>32) out.push('tall cell '+t);
      }
      for (const e of document.querySelectorAll('.win .body,.win .bar,.win .log,.win .side')) if(vis(e)&&e.scrollWidth>e.clientWidth+1) out.push('h-scroll .'+e.className);
      for (const e of document.querySelectorAll('.win .bar .tools,.win .compose')) { if(!vis(e)) continue; const r=e.getBoundingClientRect(); if(r.right>W||r.bottom>H) out.push('out of window .'+e.className); }
      const side=document.querySelector('.side'); if(vis(side)) for(const e of side.querySelectorAll('b,span,.navb')) if(vis(e)&&e.getBoundingClientRect().right>side.getBoundingClientRect().right) out.push('sidebar overflow "'+e.textContent+'"');
      return out;
    });
    if (issues.length) { bad += issues.length; console.log(`${s} ${theme} ${w}:`, [...new Set(issues)].join(' | ')); }
    await p.screenshot({ path: `${out}/${s}-${theme}-${w}x${h}.png` });
  }
  await p.close();
}
// one keyboard-focus proof shot
const p = await b.newPage({ viewport:{width:1200,height:800}, deviceScaleFactor:2 });
await p.goto(`file://${file}#funnel,light,bare`); await p.reload();
for (let i=0;i<8;i++) await p.keyboard.press('Tab');
await p.screenshot({ path: `${out}/focus-funnel-light-1200x800.png` });
await b.close();
console.log(bad ? `FAIL: ${bad} issues` : 'OK: no wrapped, clipped or overflowing text');
process.exit(bad ? 1 : 0);
