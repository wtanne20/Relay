import { forwardRef, type ReactNode } from 'react';
import {
  ActivityIndicator,
  Pressable,
  StyleSheet,
  TextInput,
  View,
  type StyleProp,
  type ViewStyle,
} from 'react-native';

import { enterToSend } from '../lib/layout';
import { voiceAvailable } from '../lib/voice';
import { AttachMenu, AttachmentStrip, type useAttachments } from './Attachments';
import { ModelChip } from './ModelPicker';
import { m3, radius, space } from './theme';
import { Icon } from './ui';

/**
 * The message box: attachments, the text field, the + menu, the model chip
 * and the round button on the right. The home screen and a chat both use it,
 * so a new button or rule only has to be added here.
 *
 * The round button is, in order: Stop while the agent works (with a "send
 * when done" button beside it once something is typed); the voice button
 * when nothing is typed and voice works here; otherwise Send.
 */
export const Composer = forwardRef<
  React.ComponentRef<typeof TextInput>,
  {
    value: string;
    onChangeText: (text: string) => void;
    onSend: () => void;
    placeholder: string;
    attachments: ReturnType<typeof useAttachments>;
    /** A send is on its way: the button spins and nothing new can be added. */
    busy: boolean;
    model: {
      value: string | null | undefined;
      fallback?: string | null;
      /** Routing picks the model: the chip says Auto. */
      auto?: boolean;
      onPress: () => void;
      disabled?: boolean;
    };
    /** Starts voice mode; the voice button shows only where voice works. */
    onTalk?: () => void;
    /** What the voice button says to screen readers. */
    talkLabel?: string;
    /** Set while the agent works on this chat: Stop replaces Send, and typed messages wait their turn. */
    running?: { onStop: () => void; stopping: boolean };
    /** Shown before the round button (for example "add to queue"). */
    extraActions?: ReactNode;
    /** Above the text field (for example the home screen's project picker). */
    header?: ReactNode;
    /** After the button row, inside the box (for example a menu that drops down over the page). */
    children?: ReactNode;
    /** What the Send button says to screen readers. */
    sendLabel?: string;
    /** The home screen's bigger box; a chat uses the compact one. */
    roomy?: boolean;
    /** With `roomy`: a phone-sized text field. */
    phone?: boolean;
    autoFocus?: boolean;
    style?: StyleProp<ViewStyle>;
  }
>(function Composer(
  {
    value,
    onChangeText,
    onSend,
    placeholder,
    attachments,
    busy,
    model,
    onTalk,
    talkLabel = 'Talk in voice mode',
    running,
    extraActions,
    header,
    children,
    sendLabel = 'Send',
    roomy,
    phone,
    autoFocus,
    style,
  },
  ref,
) {
  const canSend = (!!value.trim() || attachments.items.length > 0) && !busy;
  const canTalk = !!onTalk && voiceAvailable() && !canSend && !busy;

  return (
    <View style={[styles.box, roomy && styles.boxRoomy, style]}>
      {header}
      <AttachmentStrip items={attachments.items} onRemove={attachments.remove} />
      <TextInput
        ref={ref}
        style={[styles.input, roomy && styles.inputRoomy, roomy && phone && styles.inputRoomyPhone]}
        value={value}
        onChangeText={onChangeText}
        onKeyPress={enterToSend(onSend)}
        placeholder={placeholder}
        placeholderTextColor={m3.outline}
        selectionColor={m3.primary}
        cursorColor={m3.primary}
        multiline
        autoCapitalize="sentences"
        autoFocus={autoFocus}
        accessibilityLabel="Prompt"
      />
      <View style={[styles.row, roomy && styles.rowRoomy]}>
        <View style={styles.tools}>
          <AttachMenu onPick={attachments.pick} disabled={busy} />
          <ModelChip
            model={model.value}
            fallback={model.fallback}
            auto={model.auto}
            onPress={model.onPress}
            disabled={model.disabled}
          />
        </View>
        {extraActions}
        {running && value.trim() ? (
          <RoundButton
            icon="clock-plus-outline"
            label="Send when the agent is done"
            onPress={onSend}
            disabled={busy}
            busy={busy}
            tone="queue"
          />
        ) : null}
        {running ? (
          <RoundButton
            icon="stop"
            label="Stop the agent"
            onPress={running.onStop}
            disabled={running.stopping}
            busy={running.stopping}
            tone="stop"
          />
        ) : canTalk ? (
          <RoundButton icon="account-voice" label={talkLabel} onPress={onTalk} tone="on" />
        ) : (
          <RoundButton
            icon="arrow-up"
            label={sendLabel}
            onPress={onSend}
            disabled={!canSend}
            busy={busy}
            tone={canSend ? 'on' : 'off'}
          />
        )}
      </View>
      {children}
    </View>
  );
});

const tones = {
  on: { bg: m3.primary, fg: m3.onPrimary },
  off: { bg: m3.surfaceContainerHighest, fg: m3.outline },
  queue: { bg: m3.secondaryContainer, fg: m3.onSecondaryContainer },
  stop: { bg: m3.errorContainer, fg: m3.onErrorContainer },
} as const;

function RoundButton({
  icon,
  label,
  onPress,
  disabled,
  busy,
  tone,
}: {
  icon: 'arrow-up' | 'stop' | 'account-voice' | 'clock-plus-outline';
  label: string;
  onPress?: () => void;
  disabled?: boolean;
  busy?: boolean;
  tone: keyof typeof tones;
}) {
  const { bg, fg } = tones[tone];
  // While a send is on its way the button stays dark, with a light spinner.
  const spinner = tone === 'off' ? m3.onPrimary : fg;
  return (
    <Pressable
      onPress={onPress}
      disabled={disabled}
      style={({ pressed }) => [
        styles.round,
        { backgroundColor: busy && tone === 'off' ? m3.primary : bg },
        tone === 'stop' && styles.stop,
        pressed && styles.pressed,
      ]}
      accessibilityRole="button"
      accessibilityLabel={label}
    >
      {busy ? <ActivityIndicator color={spinner} /> : <Icon name={icon} size={22} color={fg} />}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  box: {
    backgroundColor: m3.surfaceContainerHigh,
    borderRadius: radius.xl,
    paddingVertical: space.sm,
    paddingHorizontal: space.sm,
  },
  boxRoomy: {
    padding: space.md,
    zIndex: 10,
    borderWidth: 1,
    borderColor: m3.outlineVariant,
  },
  input: {
    minHeight: 44,
    maxHeight: 180,
    color: m3.onSurface,
    fontSize: 16,
    lineHeight: 22,
    paddingHorizontal: space.md,
    paddingTop: space.sm,
    paddingBottom: space.xs,
  },
  inputRoomy: { minHeight: 56, maxHeight: 320, fontSize: 17, lineHeight: 25, paddingBottom: 0 },
  inputRoomyPhone: { minHeight: 44, fontSize: 16, lineHeight: 22 },
  row: { flexDirection: 'row', alignItems: 'center', gap: space.sm },
  rowRoomy: { marginTop: space.sm },
  tools: { flexDirection: 'row', alignItems: 'center', gap: 4, flex: 1, minWidth: 0 },
  round: { width: 44, height: 44, borderRadius: 22, alignItems: 'center', justifyContent: 'center' },
  stop: { borderRadius: 14 },
  pressed: { opacity: 0.85 },
});
