import { createRouter, createWebHistory } from "vue-router";
import { trackNavStart, trackNavEnd, trackError } from "./loadingBar";
import { setTransitionName } from "./composables/usePageTransition";
import { openDownloadsDrawer } from "./composables/useDownloadsDrawer";

const router = createRouter({
  history: createWebHistory(),
  routes: [
    { path: "/", name: "home", component: () => import("./views/HomeView.vue"), meta: { navSection: "home", titleKey: "nav.home", icon: "home" } },
    // 新闻已并入「发现」页（右上角入口 + 抽屉），旧地址 /news 重定向到 /browse
    { path: "/news", name: "news", redirect: "/browse" },
    { path: "/browse", name: "browse", component: () => import("./views/BrowseView.vue"), meta: { navSection: "browse", titleKey: "nav.browse", icon: "compass" } },
    { path: "/project/:provider/:id", name: "project-detail", component: () => import("./views/ProjectDetailView.vue"), meta: { navSection: "browse", detail: true, titleKey: "page.projectDetail", icon: "compass" } },
    { path: "/instances", name: "instances", component: () => import("./views/InstancesView.vue"), meta: { navSection: "instances", titleKey: "nav.instances", icon: "grid", action: { textKey: "page.newInstance", icon: "plus", to: "/create" } } },
    { path: "/instance/:id", name: "instance", component: () => import("./views/InstanceDetailView.vue"), meta: { navSection: "instances", detail: true, titleKey: "page.instanceDetail", icon: "grid" } },
    { path: "/create", name: "create", component: () => import("./views/CreateInstanceView.vue"), meta: { navSection: "instances", detail: true, titleKey: "page.create", icon: "plus" } },
    { path: "/multiplayer", name: "multiplayer", component: () => import("./views/MultiplayerView.vue"), meta: { navSection: "multiplayer", titleKey: "nav.multiplayer", icon: "users" } },
    { path: "/multiplayer/:id", name: "server-detail", component: () => import("./views/ServerDetailView.vue"), meta: { navSection: "multiplayer", detail: true, titleKey: "page.serverDetail", icon: "users" } },
    { path: "/settings", name: "settings", component: () => import("./views/SettingsView.vue"), meta: { navSection: "settings", titleKey: "nav.settings", icon: "settings" } },
    { path: "/toolbox", name: "toolbox", component: () => import("./views/ToolboxView.vue"), meta: { navSection: "toolbox", titleKey: "nav.toolbox", icon: "tool" } },
    { path: "/toolbox/seed", name: "toolbox-seed", component: () => import("./views/SeedMapView.vue"), meta: { navSection: "toolbox", detail: true, titleKey: "page.seedMap", icon: "tool" } },
    { path: "/toolbox/schematic", name: "toolbox-schematic", component: () => import("./views/SchematicPreviewView.vue"), meta: { navSection: "toolbox", detail: true, titleKey: "page.schematicPreview", icon: "tool" } },
    { path: "/toolbox/resourcepack", name: "toolbox-resourcepack", component: () => import("./views/ResourcePackPreviewView.vue"), meta: { navSection: "toolbox", detail: true, titleKey: "page.resourcePackPreview", icon: "tool" } },
    { path: "/toolbox/shimu", name: "toolbox-shimu", component: () => import("./views/ShimuLibraryView.vue"), meta: { navSection: "toolbox", detail: true, titleKey: "shimu.libraryTitle", icon: "tool" } },
    { path: "/toolbox/nbt", name: "toolbox-nbt", component: () => import("./views/NBTEditorView.vue"), meta: { navSection: "toolbox", detail: true, titleKey: "nbt.nbtTitle", icon: "tool" } },
    { path: "/timemachine", name: "timemachine", component: () => import("./views/TimeMachineView.vue"), meta: { navSection: "timemachine", titleKey: "nav.timemachine", icon: "clock" } },
    { path: "/skins", name: "skins", component: () => import("./views/SkinView.vue"), meta: { navSection: "skins", titleKey: "nav.skins", icon: "user" } },
    // 下载中心已改为右抽屉（无独立页面）：旧地址 /downloads 重定向首页并打开抽屉
    { path: "/downloads", name: "downloads", redirect: () => {
      openDownloadsDrawer();
      return { path: "/", replace: true };
    } },
  ],
});

// —— 页面加载指示 ——
// 顶部加载条只服务于「网络请求」（见 api.ts）与「页面加载」两种场景：
// 这里让路由切换（含懒加载页面 chunk）期间显示加载条。
router.beforeEach((to, from) => {
  if (to.fullPath !== from.fullPath) {
    trackNavStart();
    setTransitionName(from.meta, to.meta);
  }
  return true;
});

// 导航结束（成功、失败或重定向）都收尾，trackNavEnd 幂等
router.afterEach(() => {
  trackNavEnd();
});
router.onError(() => {
  trackError();
  trackNavEnd();
});

export default router;
