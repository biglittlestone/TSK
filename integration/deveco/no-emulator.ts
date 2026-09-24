// 常驻：永不允许启动 HarmonyOS 模拟器/设备/真机（用户强制，on/off 都拦）。
// 拦所有工具（不只看 bash）——模型常经原生工具（非 bash）拉模拟器。
export default {
  id: "tsk-deveco-no-emulator",
  async server(_: any) {
    const LAUNCH = /(emulator\.exe|"emulator"|qemu|--device\b|-avd\b|\bavd\b|bootmode|-start\b|matebook|snapshot)/i
    return {
      "tool.execute.before": async (input: any, output: any) => {
        const args = output?.args ?? {}
        const sig = JSON.stringify(args).toLowerCase()
        if (!LAUNCH.test(sig)) return
        const blocked = `echo "[TSK 拦截: 永不启动模拟器/设备/真机 (检测到模拟器启动特征)。仅构建，不上设备。]"`
        if (typeof args.command === "string") args.command = blocked
        else if (typeof args.cmd === "string") args.cmd = blocked
        else if (args.command !== undefined) output.args = { ...args, command: blocked }
        // 兜底：把常见启动目标字段清空
        for (const k of ["device", "avd", "name", "target", "emulator"]) if (args[k] !== undefined) args[k] = ""
      },
    }
  },
}
