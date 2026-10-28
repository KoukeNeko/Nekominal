document.querySelectorAll('[data-copy]').forEach(box => {
  const btn = box.querySelector('button');
  const idle = btn.textContent;
  btn.addEventListener('click', async () => {
    try { await navigator.clipboard.writeText(box.dataset.copy); btn.textContent = btn.dataset.done ?? '已複製'; }
    catch { btn.textContent = btn.dataset.failed ?? '請手動複製'; }
    setTimeout(() => (btn.textContent = idle), 1800);
  });
});

if (window.gsap && !matchMedia('(prefers-reduced-motion: reduce)').matches) {
  gsap.registerPlugin(ScrollTrigger);
  // hero: window and cards drift apart at different speeds while scrolling
  const hero = { trigger: '.stage', start: 'top 80%', end: 'bottom top', scrub: true };
  gsap.from('.hero h1, .hero .sub, .hero .cmd', { y: 28, opacity: 0, duration: .8, stagger: .12, ease: 'power3.out' });
  gsap.from('.win', { y: 60, opacity: 0, duration: 1, delay: .3, ease: 'power3.out' });
  gsap.to('.c1', { y: -70, scrollTrigger: hero });
  gsap.to('.c2', { y: -120, scrollTrigger: hero });
  gsap.to('.c3', { y: -40, scrollTrigger: hero });
  gsap.to('.win', { y: -30, scale: .97, scrollTrigger: hero });
  // sections: reveal as each enters
  ScrollTrigger.batch('.rv', {
    start: 'top 88%', once: true,
    onEnter: els => gsap.fromTo(els, { y: 32, opacity: 0 }, { y: 0, opacity: 1, duration: .7, stagger: .09, ease: 'power3.out' }),
  });
  gsap.set('.rv', { opacity: 0 });
}
