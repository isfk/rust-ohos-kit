import { AbilityConstant, Configuration, ConfigurationConstant, UIAbility, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import { hilog } from '@kit.PerformanceAnalysisKit';
import { window } from '@kit.ArkUI';
import { appLife } from '../state/AppLife';
import { setBenchRuns } from '../core/bench';

const DOMAIN = 0x0000;

export default class EntryAbility extends UIAbility {
  private mainWin: window.Window | undefined = undefined;

  onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {
    hilog.info(DOMAIN, 'testTag', '%{public}s', 'Ability onCreate');
    // Foreground/background is an ability-level fact; components that hold
    // something expensive (a camera, a sensor) watch the same AppLife object.
    appLife().foreground = true;

    // Follow the system. Both palettes live in resources/{base,dark}, and ArkUI
    // re-resolves resource colours when the mode changes — nothing to rebuild.
    // (An earlier build pinned COLOR_MODE_DARK here, which is why the app once
    // stayed dark on a light phone: "dark by default" is not the same thing as
    // "follow the system".)
    try {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_NOT_SET);
    } catch (err) {
      hilog.error(DOMAIN, 'testTag', 'Failed to set colorMode. Cause: %{public}s', JSON.stringify(err));
    }

    // `aa start --pi bench N` asks for measurements without a finger on the
    // button (see ../../scripts/bench.sh). Anything else leaves it at zero.
    const params = want.parameters;
    if (params !== undefined) {
      const raw = params['bench'];
      if (typeof raw === 'number') {
        setBenchRuns(raw as number);
      }
    }
  }

  /** The system's current mode. The palettes follow it, so the bars have to. */
  private isDark(): boolean {
    return this.context.config.colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK;
  }

  onDestroy(): void {
    hilog.info(DOMAIN, 'testTag', '%{public}s', 'Ability onDestroy');
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    // Main window is created, set main page for this ability
    hilog.info(DOMAIN, 'testTag', '%{public}s', 'Ability onWindowStageCreate');

    windowStage.loadContent('pages/Index', (err) => {
      if (err.code) {
        hilog.error(DOMAIN, 'testTag', 'Failed to load the content. Cause: %{public}s', JSON.stringify(err));
        return;
      }
      hilog.info(DOMAIN, 'testTag', 'Succeeded in loading the content.');
    });

    // Immersive: the page draws under the system bars, and both bars turn
    // transparent so there is something to draw under. The page then reads the
    // avoid areas back and pads itself — a status bar and a gesture bar are
    // different heights, and hard-coding them is how content ends up under the
    // clock.
    //
    // setImmersiveModeEnabledState (API 12) and setWindowLayoutFullScreen
    // (API 9) are the same switch under two names — the getter reads the value
    // either of them set. The newer name is the one kept; calling both would be
    // the kind of "belt and braces" that hides which one works.
    try {
      const win = windowStage.getMainWindowSync();
      this.mainWin = win;
      win.setImmersiveModeEnabledState(true);
      this.applyBars(win, this.isDark());
      hilog.info(DOMAIN, 'testTag', 'immersive state=%{public}s', `${win.getImmersiveModeEnabledState()}`);
      try {
        hilog.info(DOMAIN, 'testTag', 'isImmersiveLayout=%{public}s', `${win.isImmersiveLayout()}`);
      } catch (err) {
        hilog.warn(DOMAIN, 'testTag', 'isImmersiveLayout unavailable: %{public}s', JSON.stringify(err));
      }
    } catch (err) {
      hilog.error(DOMAIN, 'testTag', 'Failed to go immersive. Cause: %{public}s', JSON.stringify(err));
    }
  }

  /**
   * Transparent bars, and glyphs that can be read against the page's own
   * background. The bars are see-through, so "white glyphs on a dark page" is the
   * whole rule — there is no bar colour to match.
   */
  private applyBars(win: window.Window, dark: boolean): void {
    win.setWindowSystemBarProperties({
      statusBarColor: '#00000000',
      navigationBarColor: '#00000000',
      statusBarContentColor: dark ? '#FFFFFF' : '#000000',
      navigationBarContentColor: dark ? '#FFFFFF' : '#000000',
    }).catch((err: BusinessError) => {
      hilog.error(DOMAIN, 'testTag', 'system bar properties failed: %{public}s', JSON.stringify(err));
    });
  }

  /**
   * When the system flips, the glyphs have to flip with it or the clock turns
   * invisible against a transparent bar. The page's own colours switch by
   * themselves (they are resources); this is the one thing that isn't.
   */
  onConfigurationUpdate(newConfig: Configuration): void {
    if (this.mainWin === undefined) {
      return;
    }
    this.applyBars(this.mainWin, newConfig.colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK);
  }

  onWindowStageDestroy(): void {
    // Main window is destroyed, release UI related resources
    this.mainWin = undefined;
    hilog.info(DOMAIN, 'testTag', '%{public}s', 'Ability onWindowStageDestroy');
  }

  onForeground(): void {
    // Ability has brought to foreground
    hilog.info(DOMAIN, 'testTag', '%{public}s', 'Ability onForeground');
    appLife().foreground = true;
  }

  onBackground(): void {
    // Ability has back to background
    hilog.info(DOMAIN, 'testTag', '%{public}s', 'Ability onBackground');
    // Not onPageHide — a page that goes behind another still exists, and a
    // camera left previewing in the background is exactly the bug.
    appLife().foreground = false;
  }
}
