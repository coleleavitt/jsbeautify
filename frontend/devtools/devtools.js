browser.devtools.panels
  .create("Mindemon Dump", "", "/panel/panel.html")
  .catch(error => console.error("Mindemon panel create failed", error));
