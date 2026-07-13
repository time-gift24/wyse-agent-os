import { type RouteConfig, index, route } from "@react-router/dev/routes"

export default [
  index("routes/home.tsx"),
  route("longzhong", "routes/longzhong.tsx"),
  route("ontology", "routes/ontology.tsx"),
] satisfies RouteConfig
