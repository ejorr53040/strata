function init() {
  render();
  setup();
  localStorage.getItem('k');
  helper();
}

function wire() {
  document.querySelectorAll('.item').forEach((el) => {
    el.remove();
  });
}
