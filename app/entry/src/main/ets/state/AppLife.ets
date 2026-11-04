import { AppStorageV2 } from '@kit.ArkUI';

// App-wide state, the V2 way: one @ObservedV2 class, shared through
// AppStorageV2 instead of AppStorage's string keys.
//
// The ability is not a component, so it can't own @Local state — but it can
// reach the same object, and @Trace is what makes a change here reach the UI.

@ObservedV2
export class AppLife {
  /** False while the ability is in the background: stop cameras and sensors. */
  @Trace foreground: boolean = true;
}

/** The one AppLife for this process (created on first use). */
export function appLife(): AppLife {
  return AppStorageV2.connect(AppLife, () => new AppLife())!;
}
