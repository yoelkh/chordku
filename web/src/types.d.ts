declare module "signalsmith-stretch" {
  const SignalsmithStretch: (ctx: AudioContext, options?: AudioWorkletNodeOptions) => Promise<any>;
  export default SignalsmithStretch;
}
