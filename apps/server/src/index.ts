import { handleRequest } from "./router";

export { SpaceObject } from "./space";

export default {
  // The only place the real clock is read.
  fetch: (request, env) => handleRequest(request, env, Date.now()),
} satisfies ExportedHandler<Env>;
