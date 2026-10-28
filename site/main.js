document.querySelectorAll('[data-copy]').forEach(box => {
  const btn = box.querySelector('button');
  btn.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(box.dataset.copy); btn.textContent = i18n.say('copied'); }
    catch { btn.textContent = i18n.say('copyFailed'); }
    setTimeout(() => (btn.textContent = i18n.say('copy')), 1800);
  });
});

const nav = document.getElementById('nav');
const onScroll = () => nav.classList.toggle('scrolled', scrollY > 8);
addEventListener('scroll', onScroll, { passive: true }); onScroll();

if (window.gsap && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
  gsap.registerPlugin(ScrollTrigger);
  const layers = gsap.utils.toArray('.pl'), last = layers.length - 1;

  // the four layers (and the one that is not there) start on top of each other and come apart
  const idle = [];   // what keeps moving by itself; it stops while the hero is out of view
  gsap.from(layers, { y: i => (last - i) * 150, duration: 1.3, ease: 'power3.out', onComplete: startIdle });
  gsap.from('.stack .lab', { opacity: 0, duration: .5, delay: 1.1, stagger: .09 });
  gsap.from('.hero h1, .hero .sub, .hero .cmd, .hero .total', { y: 24, opacity: 0, duration: .8, stagger: .1, ease: 'power3.out' });
  // the layer that is not there keeps marching: its outline never settles
  idle.push(gsap.to('.stack .ghost', { strokeDashoffset: -32, duration: 2.4, ease: 'none', repeat: -1 }));
  // now and then it tries to take shape, and nothing is there
  idle.push(gsap.fromTo('.stack .ghost', { fillOpacity: 0 }, { fillOpacity: .14, duration: .6, ease: 'sine.inOut', yoyo: true, repeat: -1, repeatDelay: 3.4, delay: 2.6 }));

  function startIdle() {
    // the layers float, each at its own pace
    layers.forEach((g, i) => idle.push(gsap.to(g, { y: '+=5', duration: 2.2 + i * .35, ease: 'sine.inOut', yoyo: true, repeat: -1, delay: i * .25 })));
    // the window runs commands: it types one, the keystrokes go down through ConPTY to the shell, the output comes back up
    // through ConPTY, Direct3D draws it, and the window shows it
    const plates = gsap.utils.toArray('.stack .plate');
    const ty = document.querySelector('.term .ty'), cur = document.querySelector('.term .cur');
    const out = [document.querySelector('.term .o1'), document.querySelector('.term .o2')];
    const CH = 5.4;   // width of a character of the terminal, in the units of the plate
    const runs = [
      { cmd: 'git status', out: [['On branch master', ''], ['clean', 'g']] },
      { cmd: 'cargo test', out: [['running tests...', ''], ['test result: ok.', 'g']] },
      { cmd: 'claude', out: [['* Working', 'b'], ['? Waiting', 'a']] },
    ];
    const lit = k => gsap.fromTo(plates[k], { stroke: '#9bf05b' }, { stroke: '#cdd8e3', duration: .8, ease: 'power1.out', clearProps: 'stroke' });
    const clear = () => { ty.textContent = ''; cur.setAttribute('x', 5 + 2 * CH); out.forEach(o => { o.textContent = ''; o.setAttribute('class', 'o'); }); };
    const session = gsap.timeline({ repeat: -1, delay: .8 });
    runs.forEach(r => {
      const typed = { n: 0 };
      session
        .call(clear)
        .to(typed, { n: r.cmd.length, duration: r.cmd.length * .09, ease: 'none', onUpdate() {
          const n = Math.floor(typed.n); ty.textContent = r.cmd.slice(0, n); cur.setAttribute('x', 5 + (2 + n) * CH); } })
        // Enter: the window, ConPTY and the shell light their edges in turn; the output goes back up the same way
        .add(() => lit(0), '+=.25')
        .add(() => lit(2), '+=.45')
        .add(() => lit(3), '+=.4')
        .add(() => lit(2), '+=.7')
        .add(() => { lit(1); gsap.fromTo('.stack .flash', { opacity: .35 }, { opacity: 0, duration: .6, ease: 'power1.out' }); }, '+=.4')
        .add(() => lit(0), '+=.4')
        .call(() => { out[0].textContent = r.out[0][0]; out[0].setAttribute('class', 'o ' + r.out[0][1]); }, null, '+=.15')
        .call(() => { out[1].textContent = r.out[1][0]; out[1].setAttribute('class', 'o ' + r.out[1][1]); }, null, '+=.4')
        .to({}, { duration: 2.6 });
    });
    idle.push(session, gsap.to(cur, { opacity: 0, duration: .5, repeat: -1, yoyo: true, ease: 'steps(1)' }));
    ScrollTrigger.create({ trigger: '.hero', start: 'top bottom', end: 'bottom top', onToggle: s => idle.forEach(t => t.paused(!s.isActive)) });
  }

  // pointing at a layer lifts it and puts the others back
  const stack = document.querySelector('.stack'), cap = document.getElementById('cap');
  const capKeys = ['cap.g', 'cap.1', 'cap.2', 'cap.3', 'cap.4'];
  layers.forEach((g, i) => {
    const lift = g.querySelector('.pl-hv');
    g.addEventListener('pointerenter', () => {
      stack.classList.add('hovering'); g.classList.add('on'); cap.classList.add('on'); cap.innerHTML = i18n.say(capKeys[i]);
      gsap.to(lift, { y: -16, duration: .3, ease: 'power2.out' });
    });
    g.addEventListener('pointerleave', () => {
      stack.classList.remove('hovering'); g.classList.remove('on'); cap.classList.remove('on'); cap.innerHTML = i18n.say('cap.0');
      gsap.to(lift, { y: 0, duration: .45, ease: 'power3.out' });
    });
  });
  // scrolling away from the hero pulls the layers further apart
  gsap.to('.pl-in', { y: i => i * 36, ease: 'none', scrollTrigger: { trigger: '.hero', start: 'top top', end: 'bottom top', scrub: true } });

  // the measurements draw themselves to length when they come into view
  gsap.utils.toArray('.track i').forEach(bar =>
    gsap.from(bar, { scaleX: 0, duration: .9, ease: 'power3.out', scrollTrigger: { trigger: bar, start: 'top 92%', once: true } }));

  ScrollTrigger.batch('.rv', {
    start: 'top 90%', once: true,
    onEnter: els => gsap.fromTo(els, { y: 28, opacity: 0 }, { y: 0, opacity: 1, duration: .7, stagger: .09, ease: 'power3.out' }),
  });
  gsap.set('.rv', { opacity: 0 });
}
