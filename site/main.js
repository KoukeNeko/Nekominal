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
    // a point runs down through the layers, and each one lights its edge as it passes
    const dot = document.querySelector('.stack .pk'), plates = gsap.utils.toArray('.stack .plate');
    const run = gsap.timeline({ repeat: -1, repeatDelay: 2.4, delay: .6 }).set(dot, { attr: { cy: 240 }, opacity: 0 }).to(dot, { opacity: 1, duration: .2 });
    [300, 450, 600, 750].forEach((y, k) => run
      .to(dot, { attr: { cy: y }, duration: .55, ease: 'power2.inOut' })
      .fromTo(plates[k], { stroke: '#9bf05b' }, { stroke: '#cdd8e3', duration: .8, ease: 'power1.out', clearProps: 'stroke' }, '<.45'));
    run.to(dot, { opacity: 0, duration: .3 });
    idle.push(run);
    ScrollTrigger.create({ trigger: '.hero', start: 'top bottom', end: 'bottom top', onToggle: s => idle.forEach(t => t.paused(!s.isActive)) });
  }

  // pointing at a layer lifts it and puts the others back
  const stack = document.querySelector('.stack');
  layers.forEach((g, i) => {
    const lift = g.querySelector('.pl-hv');
    g.addEventListener('pointerenter', () => { stack.classList.add('hovering'); g.classList.add('on'); gsap.to(lift, { y: -16, duration: .3, ease: 'power2.out' }); });
    g.addEventListener('pointerleave', () => { stack.classList.remove('hovering'); g.classList.remove('on'); gsap.to(lift, { y: 0, duration: .45, ease: 'power3.out' }); });
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
