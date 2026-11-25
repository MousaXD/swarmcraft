package dev.swarmcraft.fabric.mixin;

import dev.swarmcraft.fabric.OwnerlessPermissionGuard;
import java.net.SocketAddress;
import java.util.Optional;
import net.minecraft.network.chat.Component;
import net.minecraft.server.permissions.LevelBasedPermissionSet;
import net.minecraft.server.players.NameAndId;
import net.minecraft.server.players.PlayerList;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(PlayerList.class)
abstract class OwnerlessPlayerListMixin {
    @Inject(method = "isOp", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$noOp(NameAndId player, CallbackInfoReturnable<Boolean> cir) {
        if (OwnerlessPermissionGuard.active()) {
            cir.setReturnValue(false);
        }
    }

    @Inject(method = "op(Lnet/minecraft/server/players/NameAndId;)V", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyOp(NameAndId player, CallbackInfo ci) {
        if (OwnerlessPermissionGuard.active()) {
            ci.cancel();
        }
    }

    @Inject(method = "op(Lnet/minecraft/server/players/NameAndId;Ljava/util/Optional;Ljava/util/Optional;)V",
        at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyExplicitOp(NameAndId player, Optional<LevelBasedPermissionSet> permissions,
            Optional<Boolean> bypass, CallbackInfo ci) {
        if (OwnerlessPermissionGuard.active()) {
            ci.cancel();
        }
    }

    @Inject(method = "setAllowCommandsForAllPlayers", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$denyAllowCommands(boolean allow, CallbackInfo ci) {
        if (OwnerlessPermissionGuard.active() && allow) {
            ci.cancel();
        }
    }

    @Inject(method = "canPlayerLogin", at = @At("HEAD"), cancellable = true)
    private void swarmcraft$noLiveAdmission(SocketAddress address, NameAndId player,
            CallbackInfoReturnable<Component> cir) {
        if (OwnerlessPermissionGuard.active()) {
            cir.setReturnValue(Component.literal("Ownerless research only: live admission is not authorized"));
        }
    }
}
