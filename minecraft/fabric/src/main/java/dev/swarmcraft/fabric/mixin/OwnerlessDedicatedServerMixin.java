package dev.swarmcraft.fabric.mixin;

import dev.swarmcraft.fabric.OwnerlessPermissionGuard;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.server.dedicated.DedicatedServer;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(DedicatedServer.class)
abstract class OwnerlessDedicatedServerMixin {
    @Inject(method = "initServer", at = @At("HEAD"))
    private void swarmcraft$checkOwnerlessSettings(CallbackInfoReturnable<Boolean> cir) {
        OwnerlessPermissionGuard.checkSettings(((DedicatedServer) (Object) this).getProperties());
    }

    @Inject(method = "handleConsoleInput", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyConsole(String command, CommandSourceStack source, CallbackInfo ci) {
        if (OwnerlessPermissionGuard.active()) {
            ci.cancel();
        }
    }

    @Inject(method = "runCommand", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyRcon(String command, CallbackInfoReturnable<String> cir) {
        if (OwnerlessPermissionGuard.active()) {
            cir.setReturnValue("Ownerless permission lab: privileged console disabled");
        }
    }

    @Inject(method = "setOperatorUserPermissions", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyOperatorPolicyChange(LevelBasedPermissionSet permissions, CallbackInfo ci) {
        if (OwnerlessPermissionGuard.active()) {
            ci.cancel();
        }
    }
}
