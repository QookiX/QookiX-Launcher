import { invoke } from "./invoke";
import type {
  QookixRegistration,
  QookixRegisterPolicy,
  QookixServiceStatus,
  QookixStatus,
  QookixTwoFactorChallenge,
  QookixTwoFactorSetup,
  QookixTwoFactorStatus,
} from "../types";

export const authApi = {
  qookixStatus: () => invoke<QookixStatus>("qookix_status"),
  /** 服务状态（公开接口，不需要登录） */
  qookixServiceStatus: () =>
    invoke<QookixServiceStatus>("qookix_service_status", undefined, { net: true }),
  qookixLogin: (login: string, password: string) =>
    invoke<QookixStatus | QookixTwoFactorChallenge>(
      "qookix_login",
      { login, password },
      { net: true },
    ),
  qookixLogin2fa: (challengeToken: string, code: string) =>
    invoke<QookixStatus>(
      "qookix_login_2fa",
      { challengeToken, code },
      { net: true },
    ),
  /** 注册策略：邀请码 / 人机验证 / 协议地址（公开） */
  qookixRegisterPolicy: () =>
    invoke<QookixRegisterPolicy>("qookix_register_policy", undefined, { net: true }),
  qookixRegister: (
    username: string,
    email: string,
    password: string,
    nickname: string | undefined,
    inviteCode: string | undefined,
    agreedTerms: boolean,
  ) =>
    invoke<QookixRegistration>(
      "qookix_register",
      {
        username,
        email,
        password,
        nickname: nickname ?? null,
        inviteCode: inviteCode || null,
        agreedTerms,
      },
      { net: true },
    ),
  qookix2faStatus: () =>
    invoke<QookixTwoFactorStatus>("qookix_2fa_status", undefined, { net: true }),
  qookix2faSetup: () =>
    invoke<QookixTwoFactorSetup>("qookix_2fa_setup", undefined, { net: true }),
  qookix2faEnable: (code: string) =>
    invoke<{ enabled: boolean }>("qookix_2fa_enable", { code }, { net: true }),
  qookix2faDisable: (password: string, code: string) =>
    invoke<{ enabled: boolean }>("qookix_2fa_disable", { password, code }, { net: true }),
  qookixChangePassword: (oldPassword: string, newPassword: string) =>
    invoke<void>(
      "qookix_change_password",
      { oldPassword, newPassword },
      { net: true },
    ),
  qookixResendVerification: (login: string) =>
    invoke<{ message: string }>("qookix_resend_verification", { login }, { net: true }),
  qookixForgotPassword: (login: string) =>
    invoke<{ message: string }>("qookix_forgot_password", { login }, { net: true }),
  qookixLogout: () => invoke<QookixStatus>("qookix_logout", undefined, { net: true }),
};
