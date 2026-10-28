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
  gsap.from(layers, { y: i => (last - i) * 150, duration: 1.3, ease: 'power3.out' });
  gsap.from('.stack .lab', { opacity: 0, duration: .5, delay: 1.1, stagger: .09 });
  gsap.from('.hero h1, .hero .sub, .hero .cmd, .hero .total', { y: 24, opacity: 0, duration: .8, stagger: .1, ease: 'power3.out' });
  // the layer that is not there keeps marching: its outline never settles
  gsap.to('.stack .ghost', { strokeDashoffset: -32, duration: 2.4, ease: 'none', repeat: -1 });
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
