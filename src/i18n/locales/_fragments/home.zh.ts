export default {
  home: {
    /** 未启用自定义主页时首页的默认内容 */
    default: {
      recentTitle: "最近游玩",
      pinTitle: "快捷启动",
      viewAll: "全部实例",
    },
    pin: {
      typeServer: "服务器",
      typeWorld: "存档",
      typeInstance: "实例",
      offline: "离线",
      playersOnline: "{count} 人在线",
      unpinTitle: "取消固定",
      launching: "启动中…",
      launch: "启动",
    },
    empty: {
      noInstance: "还没有游戏实例",
      createFirst: "创建第一个实例",
      onboarding: "新手引导",
    },
    resident: {
      lastPlayed: "最近",
      switchInstance: "切换实例",
      launching: "启动中…",
      launchGame: "启动游戏",
    },
    picker: {
      title: "切换实例",
      ungrouped: "未分组",
      current: "当前",
    },
    message: {
      noInstance: "还没有实例，先创建一个吧",
      addAccount: "请先在标题栏添加账号（正版或离线）",
      launched: "已启动 {name}",
      quickPlayUnsupported:
        "此实例是 {version}，不支持命令行直达存档，将启动游戏后手动进入存档",
      joiningServer: "正在加入服务器「{name}」",
      enteringWorld: "正在进入世界「{name}」",
      launchingInstance: "正在启动实例「{name}」",
    },
    /** 自定义主页（home.json）的运行时提示；配置入口在设置页 */
    layout: {
      reloaded: "布局已重新加载",
      parseError: "布局文件有误：{error}",
      openFailed: "打开失败：{error}",
      goSettings: "可在「设置 → 外观 → 自定义主页」里编辑或恢复默认。",
      copied: "已复制到剪贴板",
      copyFailed: "复制失败",
      dialogTitle: "提示",
      guest: "玩家",
      noInstanceName: "尚未选择实例",
      launchNoInstance: "还没有可启动的实例",
      launchNoAccount: "请先在标题栏添加账号（正版或离线）",
      launched: "已启动 {name}",
      unknownAction: "不支持的事件类型：{type}",
    },
  },
};
