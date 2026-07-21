chrome.runtime.sendMessage({ type: "lume:health" }, (response) => {
  const connected = Boolean(response?.ok);
  document.querySelector("#status").classList.toggle("online", connected);
  document.querySelector("#label").textContent = connected
    ? "Aplicativo conectado"
    : "Abra o aplicativo Lume";
});
