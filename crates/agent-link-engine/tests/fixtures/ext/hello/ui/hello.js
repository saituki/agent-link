(() => {
  AgentLinkExt.register("hello", {
    async mount(el, ctx) {
      const out = await ctx.cli(["greet", ctx.project]);
      el.appendChild(saiEl("p", "x-hello-text", JSON.stringify(out)));
    },
    interval: 0,
  });
})();
