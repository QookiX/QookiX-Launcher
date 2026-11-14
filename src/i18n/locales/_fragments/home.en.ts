export default {
  home: {
    /** Default home content when the custom home layout is off */
    default: {
      recentTitle: "Recent",
      pinTitle: "Quick launch",
      viewAll: "All instances",
    },
    pin: {
      typeServer: "Server",
      typeWorld: "Save",
      typeInstance: "Instance",
      offline: "Offline",
      playersOnline: "{count} online",
      unpinTitle: "Unpin",
      launching: "Launching…",
      launch: "Launch",
    },
    empty: {
      noInstance: "No instances yet",
      createFirst: "Create your first instance",
      onboarding: "Onboarding",
    },
    resident: {
      lastPlayed: "Last played",
      switchInstance: "Switch instance",
      launching: "Launching…",
      launchGame: "Launch game",
    },
    picker: {
      title: "Switch instance",
      ungrouped: "Ungrouped",
      current: "Current",
    },
    message: {
      noInstance: "No instances yet, create one first",
      addAccount: "Please add an account (Microsoft or offline) from the title bar first",
      launched: "Launched {name}",
      quickPlayUnsupported:
        "This instance is {version}, which doesn't support quick-play to saves. The game will start and you can enter the save manually.",
      joiningServer: 'Joining server "{name}"',
      enteringWorld: 'Entering world "{name}"',
      launchingInstance: 'Launching instance "{name}"',
    },
    /** Runtime messages for the custom home page; config lives in Settings */
    layout: {
      reloaded: "Layout reloaded",
      parseError: "Layout file error: {error}",
      openFailed: "Failed to open: {error}",
      goSettings: "Edit or reset it under Settings > Appearance > Custom Home.",
      copied: "Copied to clipboard",
      copyFailed: "Copy failed",
      dialogTitle: "Notice",
      guest: "Player",
      noInstanceName: "No instance selected",
      launchNoInstance: "No instance available to launch",
      launchNoAccount: "Please add an account (Microsoft or offline) from the title bar first",
      launched: "Launched {name}",
      unknownAction: "Unsupported action type: {type}",
    },
  },
};
